//! What the user asks for: output format, perceptual quality target, size limits, naming.

use serde::{Deserialize, Serialize};

/// Output format policy. The engine picks the actual codec per image from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatChoice {
    /// Smallest file among the modern web formats (AVIF, WebP lossy and lossless), per image.
    Auto,
    /// AVIF everywhere: the smallest files, every current browser.
    Avif,
    /// WebP everywhere: lossy for photos, lossless when it is smaller (logos, screenshots).
    Webp,
    /// Maximum compatibility: MozJPEG for photos, optimised PNG for transparency and graphics.
    Classic,
    /// Keep each file's format and only optimise it (HEIC, TIFF, BMP… become JPEG/PNG).
    Keep,
}

impl FormatChoice {
    pub const ALL: [FormatChoice; 5] = [Self::Auto, Self::Avif, Self::Webp, Self::Classic, Self::Keep];
}

/// How hard the encoders work. Never changes the visual target, only time vs. a few % of size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effort {
    Fast,
    Balanced,
    Max,
}

impl Effort {
    pub const ALL: [Effort; 3] = [Self::Fast, Self::Balanced, Self::Max];

    pub(crate) fn avif_speed(self) -> u8 {
        match self {
            Self::Fast => 8,
            Self::Balanced => 6,
            Self::Max => 4,
        }
    }
    pub(crate) fn webp_method(self) -> i32 {
        match self {
            Self::Fast => 4,
            Self::Balanced | Self::Max => 6,
        }
    }
    pub(crate) fn oxipng_preset(self) -> u8 {
        match self {
            Self::Fast => 2,
            Self::Balanced => 3,
            Self::Max => 5,
        }
    }
    pub(crate) fn quant_speed(self) -> i32 {
        match self {
            Self::Fast => 8,
            Self::Balanced => 4,
            Self::Max => 1,
        }
    }
    /// Stop the quality search once the bracket is this narrow (encoder quality units).
    pub(crate) fn search_tolerance(self) -> f32 {
        match self {
            Self::Fast => 4.0,
            Self::Balanced => 2.0,
            Self::Max => 1.0,
        }
    }
}

/// Named points on the SSIMULACRA2 scale (0–100, higher is closer to the original):
/// (score, name, explanation).
pub const QUALITY_PRESETS: [(f32, &str, &str); 5] = [
    (90.0, "Sans perte visible", "Indiscernable de l'original, même zoomé"),
    (85.0, "Excellente", "Aucune différence à 100 % — recommandé"),
    (80.0, "Très bonne", "Différences invisibles en usage normal"),
    (72.0, "Bonne", "Vignettes, fonds, images secondaires"),
    (62.0, "Légère", "Le plus léger, petits défauts possibles"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub format: FormatChoice,
    /// SSIMULACRA2 score every output must reach (50–95).
    pub target_score: f32,
    /// Longest side in pixels; `None` keeps the original dimensions. Never upscales.
    pub max_side: Option<u32>,
    /// Extra widths for `srcset` (e.g. 640, 1024). Only those smaller than the main image are made.
    pub responsive_widths: Vec<u32>,
    /// `Photo Équipe (1).JPG` → `photo-equipe-1.avif`.
    pub slug_names: bool,
    pub effort: Effort,
    /// Writes `imgsoft-manifest.json` (dimensions, sizes and variants of every image).
    pub write_manifest: bool,
    /// Copies files that are not raster images to compress (SVG, ICO, animations) to the output.
    pub copy_other_files: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: FormatChoice::Auto,
            target_score: 85.0,
            max_side: Some(2560),
            responsive_widths: Vec::new(),
            slug_names: true,
            effort: Effort::Balanced,
            write_manifest: false,
            copy_other_files: true,
        }
    }
}

/// Ready-made answers to "where will these images go?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    ModernWeb,
    Cms,
    Compatibility,
    Archive,
}

impl Profile {
    pub const ALL: [Profile; 4] = [Self::ModernWeb, Self::Cms, Self::Compatibility, Self::Archive];

    pub fn apply(self, s: &mut Settings) {
        match self {
            Self::ModernWeb => {
                s.format = FormatChoice::Auto;
                s.target_score = 85.0;
                s.max_side = Some(2560);
            }
            Self::Cms => {
                s.format = FormatChoice::Webp;
                s.target_score = 85.0;
                s.max_side = Some(2048);
            }
            Self::Compatibility => {
                s.format = FormatChoice::Classic;
                s.target_score = 85.0;
                s.max_side = Some(1920);
            }
            Self::Archive => {
                s.format = FormatChoice::Keep;
                s.target_score = 90.0;
                s.max_side = None;
            }
        }
    }

    pub fn matches(self, s: &Settings) -> bool {
        let mut probe = s.clone();
        self.apply(&mut probe);
        probe == *s
    }
}
