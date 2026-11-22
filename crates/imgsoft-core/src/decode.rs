//! Reading source files: format sniffing, animation detection, orientation and sRGB conversion.

use std::io::Cursor;
use std::path::Path;

use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use crate::Error;
use crate::bitmap::Bitmap;

/// What a source file contains, from its bytes (the extension is only a fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SourceFormat {
    Jpeg,
    Png,
    Webp,
    Gif,
    Avif,
    Heic,
    Jxl,
    Tiff,
    Bmp,
    Tga,
    Qoi,
    Pnm,
    Svg,
    Ico,
}

impl SourceFormat {
    /// Extensions picked up when scanning a folder.
    pub const RASTER_EXTENSIONS: &'static [&'static str] = &[
        "jpg", "jpeg", "jpe", "jfif", "png", "webp", "gif", "avif", "heic", "heif", "hif", "jxl", "tif", "tiff", "bmp",
        "tga", "qoi", "pnm", "ppm", "pgm", "pbm", "pam",
    ];
    /// Files copied as they are (already web assets that a raster re-encode would break).
    pub const COPY_EXTENSIONS: &'static [&'static str] = &["svg", "ico"];

    pub fn from_extension(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "jpg" | "jpeg" | "jpe" | "jfif" => Self::Jpeg,
            "png" => Self::Png,
            "webp" => Self::Webp,
            "gif" => Self::Gif,
            "avif" => Self::Avif,
            "heic" | "heif" | "hif" => Self::Heic,
            "jxl" => Self::Jxl,
            "tif" | "tiff" => Self::Tiff,
            "bmp" => Self::Bmp,
            "tga" => Self::Tga,
            "qoi" => Self::Qoi,
            "pnm" | "ppm" | "pgm" | "pbm" | "pam" => Self::Pnm,
            "svg" => Self::Svg,
            "ico" => Self::Ico,
            _ => return None,
        })
    }

    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
            let brand = &bytes[8..12];
            return match brand {
                b"avif" | b"avis" => Some(Self::Avif),
                b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => Some(Self::Heic),
                _ => None,
            };
        }
        if bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(b"\0\0\0\x0CJXL \r\n\x87\n") {
            return Some(Self::Jxl);
        }
        Some(match image::guess_format(bytes).ok()? {
            ImageFormat::Jpeg => Self::Jpeg,
            ImageFormat::Png => Self::Png,
            ImageFormat::WebP => Self::Webp,
            ImageFormat::Gif => Self::Gif,
            ImageFormat::Tiff => Self::Tiff,
            ImageFormat::Bmp => Self::Bmp,
            ImageFormat::Qoi => Self::Qoi,
            ImageFormat::Pnm => Self::Pnm,
            ImageFormat::Ico => Self::Ico,
            ImageFormat::Avif => Self::Avif,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Webp => "WebP",
            Self::Gif => "GIF",
            Self::Avif => "AVIF",
            Self::Heic => "HEIC",
            Self::Jxl => "JPEG XL",
            Self::Tiff => "TIFF",
            Self::Bmp => "BMP",
            Self::Tga => "TGA",
            Self::Qoi => "QOI",
            Self::Pnm => "PNM",
            Self::Svg => "SVG",
            Self::Ico => "ICO",
        }
    }

    fn image_format(self) -> Option<ImageFormat> {
        Some(match self {
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
            Self::Webp => ImageFormat::WebP,
            Self::Gif => ImageFormat::Gif,
            Self::Tiff => ImageFormat::Tiff,
            Self::Bmp => ImageFormat::Bmp,
            Self::Tga => ImageFormat::Tga,
            Self::Qoi => ImageFormat::Qoi,
            Self::Pnm => ImageFormat::Pnm,
            _ => return None,
        })
    }
}

/// True for animated GIF / APNG / WebP / AVIF sequences: those are copied untouched.
pub fn is_animated(format: SourceFormat, bytes: &[u8]) -> bool {
    match format {
        SourceFormat::Gif => image::codecs::gif::GifDecoder::new(Cursor::new(bytes))
            .map(|d| d.into_frames().take(2).count() > 1)
            .unwrap_or(false),
        SourceFormat::Png => {
            image::codecs::png::PngDecoder::new(Cursor::new(bytes)).and_then(|d| d.is_apng()).unwrap_or(false)
        }
        SourceFormat::Webp => {
            image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map(|d| d.has_animation()).unwrap_or(false)
        }
        SourceFormat::Avif => bytes.len() >= 12 && &bytes[8..12] == b"avis",
        SourceFormat::Heic => bytes.len() >= 12 && matches!(&bytes[8..12], b"msf1" | b"hevc" | b"hevx"),
        _ => false,
    }
}

/// Decodes to oriented 8-bit sRGB RGBA.
pub fn decode(format: SourceFormat, bytes: &[u8]) -> Result<Bitmap, Error> {
    // ImageIO handles every colour profile (CMYK included), HEIC/AVIF/JXL and big TIFFs.
    // PNG/GIF/WebP stay on the pure-Rust path: it keeps semi-transparent pixels exact.
    #[cfg(target_os = "macos")]
    if matches!(
        format,
        SourceFormat::Jpeg | SourceFormat::Heic | SourceFormat::Avif | SourceFormat::Jxl | SourceFormat::Tiff
    ) && let Some(b) = crate::macos::decode(bytes)
    {
        return Ok(b);
    }

    let Some(image_format) = format.image_format() else {
        return Err(Error::new(format!("{} n'est pas lisible sur ce système", format.label())));
    };
    match decode_with_image(image_format, bytes) {
        Ok(b) => Ok(b),
        Err(e) => {
            #[cfg(target_os = "macos")]
            if let Some(b) = crate::macos::decode(bytes) {
                return Ok(b);
            }
            Err(e)
        }
    }
}

fn decode_with_image(format: ImageFormat, bytes: &[u8]) -> Result<Bitmap, Error> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.no_limits();
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let icc = decoder.icc_profile().ok().flatten();
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut bitmap = Bitmap::new(w, h, rgba.into_raw());
    if let Some(icc) = icc {
        to_srgb(&mut bitmap.rgba, &icc);
    }
    Ok(bitmap)
}

/// Converts RGB data tagged with an ICC profile to sRGB (no-op for sRGB-like profiles).
fn to_srgb(rgba: &mut [u8], icc: &[u8]) {
    if contains(icc, b"sRGB") || contains(icc, &[0, b's', 0, b'R', 0, b'G', 0, b'B']) {
        return;
    }
    let Ok(src) = moxcms::ColorProfile::new_from_slice(icc) else { return };
    if src.color_space != moxcms::DataColorSpace::Rgb {
        return;
    }
    let dst = moxcms::ColorProfile::new_srgb();
    let Ok(t) = src.create_transform_8bit(
        moxcms::Layout::Rgba,
        &dst,
        moxcms::Layout::Rgba,
        moxcms::TransformOptions::default(),
    ) else {
        return;
    };
    let mut out = vec![0u8; rgba.len()];
    if t.transform(rgba, &mut out).is_ok() {
        rgba.copy_from_slice(&out);
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Decodes one of our own outputs (to measure it). Same pixels a browser would show.
pub(crate) fn decode_output(codec: crate::encode::Codec, bytes: &[u8]) -> Option<Bitmap> {
    use crate::encode::Codec;
    match codec {
        Codec::Webp | Codec::WebpLossless => {
            let img = webp::Decoder::new(bytes).decode()?;
            let (w, h) = (img.width(), img.height());
            let rgba = if img.is_alpha() {
                img.to_vec()
            } else {
                img.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()
            };
            Some(Bitmap::new(w, h, rgba))
        }
        Codec::Avif => {
            #[cfg(target_os = "macos")]
            {
                crate::macos::decode(bytes)
            }
            #[cfg(not(target_os = "macos"))]
            {
                None
            }
        }
        Codec::Jpeg | Codec::Png | Codec::PngQuantized => {
            let format = if codec == Codec::Jpeg { ImageFormat::Jpeg } else { ImageFormat::Png };
            let img = image::load_from_memory_with_format(bytes, format).ok()?.into_rgba8();
            let (w, h) = img.dimensions();
            Some(Bitmap::new(w, h, img.into_raw()))
        }
    }
}
