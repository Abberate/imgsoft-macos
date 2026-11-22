//! ImgSoft engine: turns client images into web-ready files that look identical to the source
//! (SSIMULACRA2 target) at the smallest size, in the best format for where they're going.

pub mod batch;
pub mod bitmap;
pub mod decode;
pub mod encode;
pub mod fmt;
mod jpeg_meta;
#[cfg(target_os = "macos")]
mod macos;
pub mod metric;
pub mod naming;
pub mod pipeline;
pub mod resize;
pub mod search;
pub mod settings;

pub use batch::{Event, Input, Summary};
pub use bitmap::Bitmap;
pub use decode::SourceFormat;
pub use encode::Codec;
pub use pipeline::{FileReport, Job, JobKind, Outcome};
pub use settings::{Effort, FormatChoice, Profile, QUALITY_PRESETS, Settings};

#[derive(Debug, Clone)]
pub struct Error {
    message: String,
    cancelled: bool,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), cancelled: false }
    }

    pub fn cancelled() -> Self {
        Self { message: "annulé".into(), cancelled: true }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<image::ImageError> for Error {
    fn from(e: image::ImageError) -> Self {
        Self::new(format!("lecture impossible : {e}"))
    }
}

impl From<oxipng::PngError> for Error {
    fn from(e: oxipng::PngError) -> Self {
        Self::new(format!("PNG : {e}"))
    }
}

impl From<imagequant::Error> for Error {
    fn from(e: imagequant::Error) -> Self {
        Self::new(format!("palette : {e}"))
    }
}

/// Decodes any supported file (sources and our own outputs) for previews.
pub fn load_bitmap(path: &std::path::Path) -> Result<Bitmap, Error> {
    let bytes = std::fs::read(path)?;
    let format = SourceFormat::sniff(&bytes)
        .or_else(|| SourceFormat::from_extension(path))
        .ok_or_else(|| Error::new("format d'image non reconnu"))?;
    decode::decode(format, &bytes)
}

/// Decodes an in-memory image (app artwork).
pub fn load_bitmap_from_memory(bytes: &[u8]) -> Result<Bitmap, Error> {
    let format = SourceFormat::sniff(bytes).ok_or_else(|| Error::new("format d'image non reconnu"))?;
    decode::decode(format, bytes)
}

/// Writes RGBA pixels as a PNG (screenshots, previews).
pub fn save_png(path: &std::path::Path, width: u32, height: u32, rgba: Vec<u8>) -> Result<(), Error> {
    let img = image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| Error::new("taille de tampon"))?;
    img.save_with_format(path, image::ImageFormat::Png)?;
    Ok(())
}
