//! One source file → its optimised output (+ responsive variants).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use serde::Serialize;

use crate::Error;
use crate::bitmap::Bitmap;
use crate::decode::{self, SourceFormat};
use crate::encode::{self, Codec};
use crate::metric::Reference;
use crate::resize;
use crate::search::{self, Candidate, Ctx};
use crate::settings::{FormatChoice, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum JobKind {
    /// Raster image to compress.
    Convert,
    /// Copied as is (SVG, ICO).
    Copy,
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub index: usize,
    /// Absolute path of the source file.
    pub source: PathBuf,
    /// Path shown to the user, relative to the input folder.
    pub display: PathBuf,
    /// Output folder, relative to the output root.
    pub out_dir: PathBuf,
    /// Output file name without extension (unique within `out_dir`).
    pub stem: String,
    pub kind: JobKind,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Outcome {
    Compressed,
    /// The original file (cleaned of metadata) was already the smallest.
    KeptOriginal,
    Copied,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantReport {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub codec: Codec,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileReport {
    pub index: usize,
    pub source: PathBuf,
    pub source_format: Option<SourceFormat>,
    pub original_bytes: u64,
    pub outcome: Outcome,
    /// Main output, relative to the output root.
    pub output: Option<PathBuf>,
    pub output_bytes: u64,
    pub codec: Option<Codec>,
    pub quality: Option<f32>,
    pub score: Option<f64>,
    pub source_size: Option<(u32, u32)>,
    pub output_size: Option<(u32, u32)>,
    pub variants: Vec<VariantReport>,
    pub millis: u64,
    pub note: Option<String>,
}

impl FileReport {
    fn new(job: &Job) -> Self {
        Self {
            index: job.index,
            source: job.display.clone(),
            source_format: None,
            original_bytes: job.bytes,
            outcome: Outcome::Cancelled,
            output: None,
            output_bytes: 0,
            codec: None,
            quality: None,
            score: None,
            source_size: None,
            output_size: None,
            variants: Vec::new(),
            millis: 0,
            note: None,
        }
    }

    /// Bytes saved on the main output (negative if it grew).
    pub fn saved_bytes(&self) -> i64 {
        match self.outcome {
            Outcome::Compressed | Outcome::KeptOriginal => self.original_bytes as i64 - self.output_bytes as i64,
            _ => 0,
        }
    }
}

/// Codecs to try for one image, by policy and content.
pub fn plan_codecs(choice: FormatChoice, src: SourceFormat, alpha: bool, graphic: bool) -> Vec<Codec> {
    use Codec::*;
    // `PngQuantized` stands for "best PNG": palette when it passes, lossless otherwise.
    let classic = || match (alpha, graphic) {
        (false, false) => vec![Jpeg],
        (false, true) => vec![PngQuantized, Jpeg],
        (true, _) => vec![PngQuantized],
    };
    let webp = || if graphic { vec![WebpLossless, Webp] } else { vec![Webp] };
    match choice {
        FormatChoice::Auto => {
            if graphic {
                vec![WebpLossless, Avif, Webp]
            } else {
                vec![Avif, Webp]
            }
        }
        FormatChoice::Avif => vec![Avif],
        FormatChoice::Webp => webp(),
        FormatChoice::Classic => classic(),
        FormatChoice::Keep => match src {
            SourceFormat::Png | SourceFormat::Gif => vec![PngQuantized],
            SourceFormat::Webp => webp(),
            SourceFormat::Avif => vec![Avif],
            _ => classic(),
        },
    }
}

/// Codecs allowed for the responsive variants of an image encoded with `main` (same extension).
fn family(main: Codec, graphic: bool) -> Vec<Codec> {
    match main {
        Codec::Avif => vec![Codec::Avif],
        Codec::Webp | Codec::WebpLossless => {
            if graphic {
                vec![Codec::WebpLossless, Codec::Webp]
            } else {
                vec![Codec::Webp]
            }
        }
        Codec::Jpeg => vec![Codec::Jpeg],
        Codec::Png | Codec::PngQuantized => vec![Codec::PngQuantized],
    }
}

fn source_codec(f: SourceFormat) -> Option<Codec> {
    Some(match f {
        SourceFormat::Jpeg => Codec::Jpeg,
        SourceFormat::Png => Codec::Png,
        SourceFormat::Webp => Codec::Webp,
        SourceFormat::Avif => Codec::Avif,
        _ => return None,
    })
}

pub fn process(job: &Job, s: &Settings, out_root: &Path, cancel: &AtomicBool) -> FileReport {
    let start = Instant::now();
    let mut report = FileReport::new(job);
    let result = match job.kind {
        JobKind::Copy => copy(job, out_root, &mut report, None),
        JobKind::Convert => convert(job, s, out_root, cancel, &mut report),
    };
    if let Err(e) = result {
        report.outcome = if e.is_cancelled() { Outcome::Cancelled } else { Outcome::Failed(e.to_string()) };
    }
    report.millis = start.elapsed().as_millis() as u64;
    report
}

/// Report for a job that was never started because the batch was cancelled.
pub fn process_cancelled(job: &Job) -> FileReport {
    FileReport::new(job)
}

fn copy(job: &Job, out_root: &Path, report: &mut FileReport, note: Option<&str>) -> Result<(), Error> {
    let ext = job.source.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = if ext.is_empty() { job.stem.clone() } else { format!("{}.{ext}", job.stem) };
    let rel = job.out_dir.join(name);
    let dest = out_root.join(&rel);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(&job.source, &dest)?;
    report.outcome = Outcome::Copied;
    report.output_bytes = job.bytes;
    report.output = Some(rel);
    report.note = note.map(str::to_owned);
    Ok(())
}

fn write(out_root: &Path, rel: &Path, bytes: &[u8]) -> Result<(), Error> {
    let dest = out_root.join(rel);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&dest, bytes)?;
    Ok(())
}

fn encode_best(bitmap: &Bitmap, codecs: &[Codec], s: &Settings, cancel: &AtomicBool) -> Result<Candidate, Error> {
    let reference = Reference::new(bitmap);
    let ctx = Ctx { bitmap, reference: reference.as_ref(), target: s.target_score, effort: s.effort, cancel };
    search::best_of(&ctx, codecs)?.ok_or_else(|| Error::new("aucun encodeur n'a abouti"))
}

fn convert(
    job: &Job,
    s: &Settings,
    out_root: &Path,
    cancel: &AtomicBool,
    report: &mut FileReport,
) -> Result<(), Error> {
    let bytes = std::fs::read(&job.source)?;
    let Some(format) = SourceFormat::sniff(&bytes).or_else(|| SourceFormat::from_extension(&job.source)) else {
        return Err(Error::new("format d'image non reconnu"));
    };
    report.source_format = Some(format);
    if matches!(format, SourceFormat::Svg | SourceFormat::Ico) {
        return copy(job, out_root, report, Some("copié tel quel"));
    }
    if decode::is_animated(format, &bytes) {
        return copy(job, out_root, report, Some("animation conservée telle quelle"));
    }

    let original = decode::decode(format, &bytes)?;
    report.source_size = Some((original.width, original.height));
    let (w, h) = resize::fit_longest_side(original.width, original.height, s.max_side);
    let resized = w != original.width || h != original.height;
    let mut main = if resized {
        let small = resize::resize(&original, w, h);
        drop(original); // free the full-size picture before the (memory-hungry) encoding
        small
    } else {
        original
    };
    main.clean_transparent();
    let alpha = main.has_alpha();
    let graphic = main.looks_like_graphic();

    let codecs = plan_codecs(s.format, format, alpha, graphic);
    let mut best = encode_best(&main, &codecs, s, cancel)?;
    let mut outcome = Outcome::Compressed;

    // Already-optimised web files: keep the original (metadata removed) when nothing beats it.
    if !resized
        && matches!(s.format, FormatChoice::Auto | FormatChoice::Keep)
        && let Some(codec) = source_codec(format)
    {
        let cleaned = match format {
            SourceFormat::Jpeg if !crate::jpeg_meta::needs_rotation(&bytes) => crate::jpeg_meta::strip(&bytes),
            SourceFormat::Jpeg => None,
            SourceFormat::Png => encode::png_recompress(&bytes, s.effort).ok(),
            _ => Some(bytes.clone()),
        };
        if let Some(cleaned) = cleaned.filter(|c| c.len() <= best.bytes.len()) {
            best = Candidate { codec, bytes: cleaned, quality: None, score: Some(100.0), meets_target: true };
            outcome = Outcome::KeptOriginal;
        }
    }

    let rel = job.out_dir.join(format!("{}.{}", job.stem, best.codec.extension()));
    write(out_root, &rel, &best.bytes)?;
    report.output = Some(rel);
    report.output_bytes = best.bytes.len() as u64;
    report.codec = Some(best.codec);
    report.quality = best.quality;
    report.score = best.score;
    report.output_size = Some((w, h));
    if !best.meets_target {
        report.note =
            Some(format!("objectif {:.0} non atteint (max {:.1})", s.target_score, best.score.unwrap_or_default()));
    }

    let mut widths: Vec<u32> = s.responsive_widths.iter().copied().filter(|&v| v > 0 && v < w).collect();
    widths.sort_unstable_by(|a, b| b.cmp(a));
    widths.dedup();
    let variant_codecs = family(best.codec, graphic);
    for vw in widths {
        // From the main picture: same look, and the (possibly huge) original is already freed.
        let (vw, vh) = resize::fit_width(main.width, main.height, vw);
        let mut small = resize::resize(&main, vw, vh);
        small.clean_transparent();
        let c = encode_best(&small, &variant_codecs, s, cancel)?;
        let rel = job.out_dir.join(format!("{}-{vw}w.{}", job.stem, c.codec.extension()));
        write(out_root, &rel, &c.bytes)?;
        report.variants.push(VariantReport {
            output: rel,
            width: vw,
            height: vh,
            bytes: c.bytes.len() as u64,
            codec: c.codec,
        });
    }
    report.outcome = outcome;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_plans() {
        use Codec::*;
        let p = |c, s, a, g| plan_codecs(c, s, a, g);
        assert_eq!(p(FormatChoice::Auto, SourceFormat::Jpeg, false, false), vec![Avif, Webp]);
        assert_eq!(p(FormatChoice::Classic, SourceFormat::Heic, false, false), vec![Jpeg]);
        assert_eq!(p(FormatChoice::Classic, SourceFormat::Png, true, false), vec![PngQuantized]);
        assert_eq!(p(FormatChoice::Keep, SourceFormat::Png, false, false), vec![PngQuantized]);
        assert_eq!(p(FormatChoice::Keep, SourceFormat::Tiff, false, false), vec![Jpeg]);
        assert_eq!(p(FormatChoice::Webp, SourceFormat::Png, true, true), vec![WebpLossless, Webp]);
    }
}
