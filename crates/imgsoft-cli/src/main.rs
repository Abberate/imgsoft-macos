//! `imgsoft` — the ImgSoft engine from the terminal (scripts, CI, big batches).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use imgsoft_core::{Effort, Event, FormatChoice, Input, Outcome, Settings, Summary, batch, fmt};

const HELP: &str = "\
ImgSoft — compresse des images pour le web (qualité perceptuelle garantie, taille minimale).

USAGE
  imgsoft <dossier|fichiers…> -o <dossier de sortie> [options]

OPTIONS
  -o, --output <dossier>   Dossier où écrire les images (obligatoire)
  -f, --format <format>    auto (défaut) | avif | webp | classic (JPEG/PNG) | keep
  -q, --quality <score>    Score SSIMULACRA2 visé, 50–95 (défaut 85 ; 90 = sans perte visible)
  -m, --max <px>           Côté le plus long en pixels (défaut 2560 ; 0 = taille d'origine)
  -w, --widths <liste>     Variantes responsive, ex. 640,1024,1600 (fichiers nom-640w.ext)
  -e, --effort <niveau>    fast | balanced (défaut) | max
      --no-slug            Garder les noms de fichiers d'origine
      --manifest           Écrire imgsoft-manifest.json (dimensions, srcset)
      --no-copy            Ne pas copier les SVG/ICO/animations
  -j, --jobs <n>           Images traitées en parallèle
  -h, --help               Cette aide
";

fn parse() -> Result<(Input, PathBuf, Settings, usize), String> {
    let mut args = std::env::args().skip(1);
    let mut inputs = Vec::new();
    let mut output = None;
    let mut s = Settings::default();
    let mut jobs = batch::default_workers();
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("valeur manquante pour {name}"));
        match a.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "-o" | "--output" => output = Some(PathBuf::from(value(&a)?)),
            "-f" | "--format" => {
                s.format = match value(&a)?.as_str() {
                    "auto" => FormatChoice::Auto,
                    "avif" => FormatChoice::Avif,
                    "webp" => FormatChoice::Webp,
                    "classic" | "jpeg" | "jpg" | "png" => FormatChoice::Classic,
                    "keep" => FormatChoice::Keep,
                    f => return Err(format!("format inconnu : {f}")),
                }
            }
            "-q" | "--quality" => {
                let q: f32 = value(&a)?.parse().map_err(|_| "score invalide")?;
                s.target_score = q.clamp(30.0, 98.0);
            }
            "-m" | "--max" => {
                let m: u32 = value(&a)?.parse().map_err(|_| "taille invalide")?;
                s.max_side = (m > 0).then_some(m);
            }
            "-w" | "--widths" => {
                s.responsive_widths = value(&a)?
                    .split(',')
                    .map(|w| w.trim().parse::<u32>().map_err(|_| format!("largeur invalide : {w}")))
                    .collect::<Result<_, _>>()?;
            }
            "-e" | "--effort" => {
                s.effort = match value(&a)?.as_str() {
                    "fast" => Effort::Fast,
                    "balanced" => Effort::Balanced,
                    "max" => Effort::Max,
                    e => return Err(format!("effort inconnu : {e}")),
                }
            }
            "-j" | "--jobs" => jobs = value(&a)?.parse().map_err(|_| "nombre invalide")?,
            "--no-slug" => s.slug_names = false,
            "--manifest" => s.write_manifest = true,
            "--no-copy" => s.copy_other_files = false,
            _ if a.starts_with('-') => return Err(format!("option inconnue : {a}")),
            _ => inputs.push(PathBuf::from(a)),
        }
    }
    let output = output.ok_or("indiquez un dossier de sortie avec -o")?;
    let input = match inputs.as_slice() {
        [] => return Err("indiquez un dossier ou des fichiers à compresser".into()),
        [one] if one.is_dir() => Input::Folder(one.clone()),
        _ => Input::Files(inputs),
    };
    Ok((input, output, s, jobs))
}

fn main() -> ExitCode {
    let (input, output, settings, jobs) = match parse() {
        Ok(v) => v,
        Err(e) if e.is_empty() => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("imgsoft : {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    let plan = match batch::plan(&input, &output, &settings) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("imgsoft : {e}");
            return ExitCode::FAILURE;
        }
    };
    if plan.is_empty() {
        eprintln!("imgsoft : aucune image trouvée");
        return ExitCode::FAILURE;
    }
    println!("{} fichier(s) → {}", plan.len(), output.display());
    let start = Instant::now();
    let cancel = AtomicBool::new(false);
    let reports = batch::run(&plan, &settings, &output, &cancel, jobs, &|event| {
        let Event::Finished(r) = event else { return };
        let out = r.output.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        match &r.outcome {
            Outcome::Compressed | Outcome::KeptOriginal => {
                let dims = match (r.source_size, r.output_size) {
                    (Some(a), Some(b)) if a != b => format!("{}×{} → {}×{}", a.0, a.1, b.0, b.1),
                    (_, Some(b)) => format!("{}×{}", b.0, b.1),
                    _ => String::new(),
                };
                let how = match (&r.outcome, r.codec, r.quality, r.score) {
                    (Outcome::KeptOriginal, _, _, _) => "original conservé".to_string(),
                    (_, Some(c), Some(q), Some(sc)) => format!("{} q{q:.0} · score {sc:.1}", c.label()),
                    (_, Some(c), _, Some(sc)) => format!("{} · score {sc:.1}", c.label()),
                    (_, Some(c), _, _) => c.label().to_string(),
                    _ => String::new(),
                };
                let variants = if r.variants.is_empty() {
                    String::new()
                } else {
                    format!(" · +{} variante(s)", r.variants.len())
                };
                println!(
                    "✓ {out}  {dims}  {} → {} ({})  {how}{variants}  {}{}",
                    fmt::bytes(r.original_bytes),
                    fmt::bytes(r.output_bytes),
                    fmt::change(r.original_bytes, r.output_bytes),
                    fmt::duration_ms(r.millis),
                    r.note.as_ref().map(|n| format!("  ⚠ {n}")).unwrap_or_default(),
                );
            }
            Outcome::Copied => println!("= {out}  copié ({})", r.note.as_deref().unwrap_or("tel quel")),
            Outcome::Failed(e) => println!("✗ {}  {e}", r.source.display()),
            Outcome::Cancelled => {}
        }
    });
    let s = Summary::of(&reports);
    println!(
        "\n{} image(s) : {} → {} ({}) en {}{}",
        s.compressed + s.kept_original,
        fmt::bytes(s.original_bytes),
        fmt::bytes(s.output_bytes),
        fmt::change(s.original_bytes, s.output_bytes),
        fmt::duration_ms(start.elapsed().as_millis() as u64),
        if s.variant_files > 0 {
            format!(" · {} variante(s), {}", s.variant_files, fmt::bytes(s.variant_bytes))
        } else {
            String::new()
        },
    );
    if s.failed > 0 {
        println!("{} échec(s)", s.failed);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
