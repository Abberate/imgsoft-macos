//! High-quality downscaling: Lanczos3 in linear light with premultiplied alpha
//! (no darkened fine detail, no colour fringes around transparent edges).

use fast_image_resize as fr;

use crate::bitmap::Bitmap;

/// Size that fits `max_side` on the longest edge (never enlarges).
pub fn fit_longest_side(width: u32, height: u32, max_side: Option<u32>) -> (u32, u32) {
    match max_side {
        Some(m) if width.max(height) > m && m > 0 => {
            if width >= height {
                (m, scaled(height, m, width))
            } else {
                (scaled(width, m, height), m)
            }
        }
        _ => (width, height),
    }
}

/// Size for a given target width (never enlarges).
pub fn fit_width(width: u32, height: u32, target_width: u32) -> (u32, u32) {
    if target_width >= width { (width, height) } else { (target_width, scaled(height, target_width, width)) }
}

fn scaled(value: u32, num: u32, den: u32) -> u32 {
    ((value as u64 * num as u64 + den as u64 / 2) / den as u64).max(1) as u32
}

pub fn resize(src: &Bitmap, width: u32, height: u32) -> Bitmap {
    if width == src.width && height == src.height {
        return src.clone();
    }
    resize_linear(src, width, height).unwrap_or_else(|| resize_plain(src, width, height))
}

fn resize_linear(src: &Bitmap, width: u32, height: u32) -> Option<Bitmap> {
    let mapper = fr::create_srgb_mapper();
    let src_img = fr::images::ImageRef::new(src.width, src.height, &src.rgba, fr::PixelType::U8x4).ok()?;
    let mut linear = fr::images::Image::new(src.width, src.height, fr::PixelType::U16x4);
    mapper.forward_map(&src_img, &mut linear).ok()?;

    let mut dst_linear = fr::images::Image::new(width, height, fr::PixelType::U16x4);
    let options =
        fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3)).use_alpha(true);
    fr::Resizer::new().resize(&linear, &mut dst_linear, &options).ok()?;

    let mut out = fr::images::Image::new(width, height, fr::PixelType::U8x4);
    mapper.backward_map(&dst_linear, &mut out).ok()?;
    Some(Bitmap::new(width, height, out.into_vec()))
}

fn resize_plain(src: &Bitmap, width: u32, height: u32) -> Bitmap {
    let img = image::RgbaImage::from_raw(src.width, src.height, src.rgba.clone()).expect("buffer size");
    let out = image::imageops::resize(&img, width, height, image::imageops::FilterType::Lanczos3);
    Bitmap::new(width, height, out.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_without_upscaling() {
        assert_eq!(fit_longest_side(4000, 3000, Some(2560)), (2560, 1920));
        assert_eq!(fit_longest_side(3000, 4000, Some(2560)), (1920, 2560));
        assert_eq!(fit_longest_side(800, 600, Some(2560)), (800, 600));
        assert_eq!(fit_longest_side(800, 600, None), (800, 600));
        assert_eq!(fit_width(2560, 1440, 640), (640, 360));
        assert_eq!(fit_width(500, 300, 640), (500, 300));
    }

    #[test]
    fn linear_light_downscale_keeps_average_brightness() {
        // Black/white 1-px stripes average to 50 % *light*, i.e. sRGB ≈ 188, not 128.
        let mut px = Vec::new();
        for _y in 0..16 {
            for x in 0..16 {
                let v = if x % 2 == 0 { 0 } else { 255 };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let out = resize(&Bitmap::new(16, 16, px), 4, 4);
        let mid = out.rgba[(2 * 4 + 2) * 4];
        assert!((180..=196).contains(&mid), "got {mid}");
    }
}
