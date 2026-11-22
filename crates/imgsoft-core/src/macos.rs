//! macOS ImageIO decoder: HEIC/HEIF (iPhone photos), AVIF, JPEG XL, TIFF and colour-managed JPEG.
//! Applies the EXIF orientation and converts any embedded profile (Display P3, Adobe RGB, CMYK…)
//! to sRGB, which is what browsers assume for untagged web images.

use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFString, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBlendMode, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, kCGColorSpaceSRGB,
};
use objc2_image_io::{
    CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCache,
};

use crate::bitmap::Bitmap;

const BYTE_ORDER_32_BIG: u32 = 4 << 12;

pub fn decode(bytes: &[u8]) -> Option<Bitmap> {
    let data = CFData::from_bytes(bytes);
    // SAFETY: no options dictionary.
    let source = unsafe { CGImageSource::with_data(&data, None) }?;
    // SAFETY: plain getters on a valid source.
    if unsafe { source.count() } == 0 {
        return None;
    }
    let index = unsafe { source.primary_image_index() };

    // A "thumbnail" without a maximum size is the full image, with the orientation applied.
    // SAFETY: the keys are ImageIO constants, the values CFBooleans, as the API documents.
    let keys: [&CFString; 3] = unsafe {
        [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceCreateThumbnailWithTransform,
            kCGImageSourceShouldCache,
        ]
    };
    let values = [CFBoolean::new(true), CFBoolean::new(true), CFBoolean::new(false)];
    let options = CFDictionary::from_slices(&keys, &values);
    let image = unsafe { source.thumbnail_at_index(index, Some(options.as_opaque())) }?;
    draw_srgb(&image)
}

fn draw_srgb(image: &CGImage) -> Option<Bitmap> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    if width == 0 || height == 0 {
        return None;
    }
    let alpha = CGImage::alpha_info(Some(image));
    let opaque =
        matches!(alpha, CGImageAlphaInfo::None | CGImageAlphaInfo::NoneSkipLast | CGImageAlphaInfo::NoneSkipFirst);
    // SAFETY: kCGColorSpaceSRGB is a valid colour space name.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    let mut buf = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
    let info = if opaque { CGImageAlphaInfo::NoneSkipLast.0 } else { CGImageAlphaInfo::PremultipliedLast.0 }
        | BYTE_ORDER_32_BIG;
    {
        // SAFETY: `buf` holds height rows of width*4 bytes and outlives the context (dropped below).
        let ctx =
            unsafe { CGBitmapContextCreate(buf.as_mut_ptr().cast(), width, height, 8, width * 4, Some(&space), info) }?;
        CGContext::set_blend_mode(Some(&ctx), CGBlendMode::Copy);
        let rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width as f64, height as f64));
        CGContext::draw_image(Some(&ctx), rect, Some(image));
    }
    if opaque {
        for p in buf.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
    } else {
        for p in buf.as_chunks_mut::<4>().0 {
            let a = p[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    }
    Some(Bitmap::new(width as u32, height as u32, buf))
}
