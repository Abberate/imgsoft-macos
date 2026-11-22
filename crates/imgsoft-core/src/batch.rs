//! Folder scanning, output naming and parallel processing of a whole batch.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::Serialize;

use crate::Error;
use crate::decode::SourceFormat;
use crate::naming::{sanitize, slugify};
use crate::pipeline::{self, FileReport, Job, JobKind, Outcome};
use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// A folder, scanned recursively (sub-folders are mirrored in the output).
    Folder(PathBuf),
    /// Loose files (e.g. dropped on the window).
    Files(Vec<PathBuf>),
}

pub const MANIFEST_NAME: &str = "imgsoft-manifest.json";

fn is_hidden(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))
}

fn kind_of(path: &Path, s: &Settings) -> Option<JobKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if SourceFormat::RASTER_EXTENSIONS.contains(&ext.as_str()) {
        Some(JobKind::Convert)
    } else if s.copy_other_files && SourceFormat::COPY_EXTENSIONS.contains(&ext.as_str()) {
        Some(JobKind::Copy)
    } else {
        None
    }
}

/// Lists the work: every image of the input with its unique output name.
pub fn plan(input: &Input, output_root: &Path, s: &Settings) -> Result<Vec<Job>, Error> {
    let out_canon = std::fs::canonicalize(output_root).unwrap_or_else(|_| output_root.to_path_buf());
    let mut found: Vec<(PathBuf, PathBuf, JobKind, u64)> = Vec::new(); // (abs, rel, kind, bytes)

    match input {
        Input::Folder(root) => {
            let root = std::fs::canonicalize(root)?;
            if root == out_canon {
                return Err(Error::new("le dossier de sortie doit être différent du dossier source"));
            }
            let walker = walkdir::WalkDir::new(&root).follow_links(true).sort_by_file_name().into_iter();
            for entry in
                walker.filter_entry(|e| e.depth() == 0 || (!is_hidden(e.path()) && !e.path().starts_with(&out_canon)))
            {
                let Ok(entry) = entry else { continue };
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let Some(kind) = kind_of(path, s) else { continue };
                let rel = path.strip_prefix(&root).unwrap_or(path).to_path_buf();
                let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                found.push((path.to_path_buf(), rel, kind, bytes));
            }
        }
        Input::Files(files) => {
            for path in files {
                let Some(kind) = kind_of(path, s) else { continue };
                let Ok(meta) = std::fs::metadata(path) else { continue };
                if !meta.is_file() {
                    continue;
                }
                let rel = PathBuf::from(path.file_name().unwrap_or_default());
                found.push((path.clone(), rel, kind, meta.len()));
            }
        }
    }

    // Unique output stems per output folder (photo.jpg + photo.png → photo, photo-2).
    let mut taken: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    let mut jobs = Vec::with_capacity(found.len());
    for (abs, rel, kind, bytes) in found {
        let out_dir: PathBuf = rel
            .parent()
            .map(|p| {
                p.components()
                    .filter_map(|c| c.as_os_str().to_str())
                    .map(|c| if s.slug_names { slugify(c) } else { sanitize(c) })
                    .collect()
            })
            .unwrap_or_default();
        let raw_stem = rel.file_stem().and_then(|n| n.to_str()).unwrap_or("image");
        let base = if s.slug_names { slugify(raw_stem) } else { sanitize(raw_stem) };
        let used = taken.entry(out_dir.clone()).or_default();
        let mut stem = base.clone();
        let mut n = 2;
        while !used.insert(stem.to_lowercase()) {
            stem = format!("{base}-{n}");
            n += 1;
        }
        jobs.push(Job { index: jobs.len(), source: abs, display: rel, out_dir, stem, kind, bytes });
    }
    Ok(jobs)
}

#[derive(Debug)]
pub enum Event {
    Started(usize),
    Finished(Box<FileReport>),
}

/// Sensible number of images processed at once (each one also uses several threads inside).
pub fn default_workers() -> usize {
    (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) / 2).clamp(1, 4)
}

/// Processes every job; reports come back in job order. Stops early when `cancel` is set.
pub fn run(
    jobs: &[Job],
    s: &Settings,
    output_root: &Path,
    cancel: &AtomicBool,
    workers: usize,
    on_event: &(dyn Fn(Event) + Sync),
) -> Vec<FileReport> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<FileReport>>> = Mutex::new(vec![None; jobs.len()]);
    std::thread::scope(|scope| {
        for _ in 0..workers.max(1).min(jobs.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(i) else { break };
                    let report = if cancel.load(Ordering::Relaxed) {
                        pipeline::process_cancelled(job)
                    } else {
                        on_event(Event::Started(i));
                        pipeline::process(job, s, output_root, cancel)
                    };
                    on_event(Event::Finished(Box::new(report.clone())));
                    results.lock().unwrap()[i] = Some(report);
                }
            });
        }
    });
    let reports: Vec<FileReport> = results.into_inner().unwrap().into_iter().flatten().collect();
    if s.write_manifest
        && !cancel.load(Ordering::Relaxed)
        && let Err(e) = write_manifest(output_root, &reports)
    {
        log::warn!("manifest: {e}");
    }
    reports
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub files: usize,
    pub compressed: usize,
    pub kept_original: usize,
    pub copied: usize,
    pub failed: usize,
    pub cancelled: usize,
    /// Sizes of the compressed/kept images before and after (copies excluded).
    pub original_bytes: u64,
    pub output_bytes: u64,
    pub variant_files: usize,
    pub variant_bytes: u64,
}

impl Summary {
    pub fn of(reports: &[FileReport]) -> Self {
        let mut s = Self { files: reports.len(), ..Self::default() };
        for r in reports {
            match r.outcome {
                Outcome::Compressed => s.compressed += 1,
                Outcome::KeptOriginal => s.kept_original += 1,
                Outcome::Copied => s.copied += 1,
                Outcome::Failed(_) => s.failed += 1,
                Outcome::Cancelled => s.cancelled += 1,
            }
            if matches!(r.outcome, Outcome::Compressed | Outcome::KeptOriginal) {
                s.original_bytes += r.original_bytes;
                s.output_bytes += r.output_bytes;
            }
            s.variant_files += r.variants.len();
            s.variant_bytes += r.variants.iter().map(|v| v.bytes).sum::<u64>();
        }
        s
    }

    /// Share of bytes saved, 0–1.
    pub fn saved_ratio(&self) -> f64 {
        if self.original_bytes == 0 { 0.0 } else { 1.0 - self.output_bytes as f64 / self.original_bytes as f64 }
    }
}

#[derive(Serialize)]
struct ManifestEntry<'a> {
    source: &'a Path,
    output: &'a Path,
    width: u32,
    height: u32,
    bytes: u64,
    format: &'static str,
    srcset: Vec<ManifestVariant<'a>>,
}

#[derive(Serialize)]
struct ManifestVariant<'a> {
    output: &'a Path,
    width: u32,
    height: u32,
    bytes: u64,
}

/// `imgsoft-manifest.json`: width/height for `<img>` (no layout shift) and the `srcset` files.
pub fn write_manifest(output_root: &Path, reports: &[FileReport]) -> Result<PathBuf, Error> {
    let images: Vec<ManifestEntry<'_>> = reports
        .iter()
        .filter_map(|r| {
            let (width, height) = r.output_size?;
            let mut srcset: Vec<ManifestVariant<'_>> = r
                .variants
                .iter()
                .map(|v| ManifestVariant { output: &v.output, width: v.width, height: v.height, bytes: v.bytes })
                .collect();
            srcset.push(ManifestVariant { output: r.output.as_deref()?, width, height, bytes: r.output_bytes });
            srcset.sort_by_key(|v| v.width);
            Some(ManifestEntry {
                source: &r.source,
                output: r.output.as_deref()?,
                width,
                height,
                bytes: r.output_bytes,
                format: r.codec?.extension(),
                srcset,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "generator": format!("ImgSoft {}", env!("CARGO_PKG_VERSION")),
        "images": images,
    });
    let path = output_root.join(MANIFEST_NAME);
    std::fs::create_dir_all(output_root)?;
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).map_err(|e| Error::new(e.to_string()))?)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_unique_slugged_names_and_skips_output() {
        let dir = std::env::temp_dir().join(format!("imgsoft-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("Équipe Été")).unwrap();
        std::fs::create_dir_all(src.join("out")).unwrap();
        for f in ["Photo 1.JPG", "photo 1.png", "notes.txt", ".hidden.png", "logo.svg"] {
            std::fs::write(src.join(f), b"x").unwrap();
        }
        std::fs::write(src.join("Équipe Été/IMG_01.heic"), b"x").unwrap();
        std::fs::write(src.join("out/old.png"), b"x").unwrap();

        let jobs = plan(&Input::Folder(src.clone()), &src.join("out"), &Settings::default()).unwrap();
        let names: Vec<String> = jobs.iter().map(|j| j.out_dir.join(&j.stem).display().to_string()).collect();
        assert_eq!(names, vec!["photo-1", "logo", "photo-1-2", "equipe-ete/img-01"]);
        assert_eq!(jobs[1].kind, JobKind::Copy);
        assert!(plan(&Input::Folder(src.clone()), &src, &Settings::default()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
