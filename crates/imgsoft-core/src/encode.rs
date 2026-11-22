//! The encoders, each tuned for the smallest file at a given visual quality:
//! AVIF (rav1e, 10-bit 4:4:4), WebP (libwebp, sharp YUV), MozJPEG (trellis, progressive),
//! PNG (imagequant palette + oxipng).

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::bitmap::Bitmap;
use crate::settings::Effort;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Codec {
    Avif,
    Webp,
    WebpLossless,
    Jpeg,
    /// Lossless PNG.
    Png,
    /// 8-bit palette PNG (pngquant-style), visually checked.
    PngQuantized,
}

impl Codec {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Avif => "avif",
            Self::Webp | Self::WebpLossless => "webp",
            Self::Jpeg => "jpg",
            Self::Png | Self::PngQuantized => "png",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Avif => "AVIF",
            Self::Webp => "WebP",
            Self::WebpLossless => "WebP sans perte",
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::PngQuantized => "PNG 8 bits",
        }
    }

    /// Codecs whose quality knob is searched (the others are lossless or palette-based).
    pub fn has_quality_knob(self) -> bool {
        matches!(self, Self::Avif | Self::Webp | Self::Jpeg)
    }
}

pub fn jpeg(b: &Bitmap, quality: f32, full_chroma: bool) -> Result<Vec<u8>, Error> {
    let gray = b.is_grayscale();
    let pixels = if b.has_alpha() { flatten_on_white(b) } else { b.clone() };
    let data = if gray { pixels.to_gray() } else { pixels.to_rgb() };
    let (w, h) = (b.width as usize, b.height as usize);
    let q = quality.clamp(1.0, 100.0);
    // libjpeg reports errors by unwinding through the C code.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || -> std::io::Result<Vec<u8>> {
        let space = if gray { mozjpeg::ColorSpace::JCS_GRAYSCALE } else { mozjpeg::ColorSpace::JCS_RGB };
        let mut c = mozjpeg::Compress::new(space);
        c.set_size(w, h);
        c.set_quality(q);
        c.set_progressive_mode();
        c.set_optimize_scans(true);
        c.set_optimize_coding(true);
        if !gray {
            let s = if full_chroma { (1, 1) } else { (2, 2) };
            c.set_chroma_sampling_pixel_sizes(s, s);
        }
        let mut c = c.start_compress(Vec::with_capacity(w * h / 4))?;
        c.write_scanlines(&data)?;
        c.finish()
    }))
    .map_err(|_| Error::new("échec de l'encodeur JPEG"))?
    .map_err(Error::from)
}

pub fn webp(b: &Bitmap, quality: f32, effort: Effort) -> Result<Vec<u8>, Error> {
    let mut config = webp::WebPConfig::new().map_err(|_| Error::new("configuration WebP"))?;
    config.quality = quality.clamp(0.0, 100.0);
    config.method = effort.webp_method();
    config.use_sharp_yuv = 1;
    config.alpha_quality = 100;
    config.alpha_filtering = 2;
    config.thread_level = 1;
    webp_encode(b, &config)
}

pub fn webp_lossless(b: &Bitmap, effort: Effort) -> Result<Vec<u8>, Error> {
    let mut config = webp::WebPConfig::new().map_err(|_| Error::new("configuration WebP"))?;
    config.lossless = 1;
    let (method, q) = match effort {
        Effort::Fast => (4, 50.0),
        Effort::Balanced => (5, 85.0),
        Effort::Max => (6, 100.0),
    };
    config.method = method;
    config.quality = q;
    config.thread_level = 1;
    webp_encode(b, &config)
}

fn webp_encode(b: &Bitmap, config: &webp::WebPConfig) -> Result<Vec<u8>, Error> {
    let mem = if b.has_alpha() {
        webp::Encoder::from_rgba(&b.rgba, b.width, b.height).encode_advanced(config)
    } else {
        let rgb = b.to_rgb();
        webp::Encoder::from_rgb(&rgb, b.width, b.height).encode_advanced(config)
    }
    .map_err(|e| Error::new(format!("encodeur WebP : {e:?}")))?;
    Ok(mem.to_vec())
}

pub fn avif(b: &Bitmap, quality: f32, effort: Effort) -> Result<Vec<u8>, Error> {
    let q = quality.clamp(1.0, 100.0);
    let encoder = ravif::Encoder::new()
        .with_quality(q)
        .with_alpha_quality((q + 10.0).min(100.0))
        .with_speed(effort.avif_speed())
        .with_num_threads(None);
    let img = ravif::Img::new(b.as_rgba8(), b.width as usize, b.height as usize);
    let out = encoder.encode_rgba(img).map_err(|e| Error::new(format!("encodeur AVIF : {e}")))?;
    Ok(out.avif_file)
}

fn png_options(effort: Effort) -> oxipng::Options {
    let mut opts = oxipng::Options::from_preset(effort.oxipng_preset());
    opts.strip = oxipng::StripChunks::Safe;
    opts.optimize_alpha = true;
    opts
}

pub fn png_lossless(b: &Bitmap, effort: Effort) -> Result<Vec<u8>, Error> {
    let raw =
        oxipng::RawImage::new(b.width, b.height, oxipng::ColorType::RGBA, oxipng::BitDepth::Eight, b.rgba.clone())?;
    Ok(raw.create_optimized_png(&png_options(effort))?)
}

/// Lossless re-compression of an existing PNG file (metadata stripped, pixels untouched).
pub fn png_recompress(bytes: &[u8], effort: Effort) -> Result<Vec<u8>, Error> {
    Ok(oxipng::optimize_from_memory(bytes, &png_options(effort))?)
}

/// A palette image: its look (for measuring) and how to write it.
pub struct Quantized {
    pub preview: Bitmap,
    palette: Vec<rgb::RGBA8>,
    indices: Vec<u8>,
}

pub fn quantize(b: &Bitmap, max_colors: u32, effort: Effort) -> Result<Quantized, Error> {
    let mut liq = imagequant::new();
    liq.set_speed(effort.quant_speed())?;
    liq.set_max_colors(max_colors)?;
    let mut img = liq.new_image_borrowed(b.as_rgba8(), b.width as usize, b.height as usize, 0.0)?;
    let mut res = liq.quantize(&mut img)?;
    res.set_dithering_level(1.0)?;
    let (palette, indices) = res.remapped(&mut img)?;
    let mut rgba = Vec::with_capacity(indices.len() * 4);
    for &i in &indices {
        let p = palette[i as usize];
        rgba.extend_from_slice(&[p.r, p.g, p.b, p.a]);
    }
    Ok(Quantized { preview: Bitmap::new(b.width, b.height, rgba), palette, indices })
}

pub fn png_indexed(q: &Quantized, effort: Effort) -> Result<Vec<u8>, Error> {
    let raw = oxipng::RawImage::new(
        q.preview.width,
        q.preview.height,
        oxipng::ColorType::Indexed { palette: q.palette.clone() },
        oxipng::BitDepth::Eight,
        q.indices.clone(),
    )?;
    Ok(raw.create_optimized_png(&png_options(effort))?)
}

fn flatten_on_white(b: &Bitmap) -> Bitmap {
    let mut out = b.clone();
    for p in out.rgba.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        for c in &mut p[..3] {
            *c = ((*c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
        p[3] = 255;
    }
    out
}
