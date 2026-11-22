//! SSIMULACRA2: perceptual distance to the original (100 = identical, 90 = visually lossless,
//! 80 = very high quality, 70 = high). Transparent images are judged over black *and* white.

use ssimulacra2::{LinearRgb, compute_frame_ssimulacra2};

use crate::bitmap::Bitmap;

pub struct Reference {
    width: u32,
    height: u32,
    /// Opaque images: the picture itself. Transparent: composited over black, then over white.
    views: Vec<LinearRgb>,
}

fn lut() -> &'static [f32; 256] {
    static LUT: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0f32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        }
        t
    })
}

fn linear(b: &Bitmap, background: Option<u8>) -> Option<LinearRgb> {
    let lut = lut();
    let data: Vec<[f32; 3]> = match background {
        None => b
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]])
            .collect(),
        Some(bg) => b
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let a = p[3] as u32;
                let mix = |c: u8| ((c as u32 * a + bg as u32 * (255 - a) + 127) / 255) as usize;
                [lut[mix(p[0])], lut[mix(p[1])], lut[mix(p[2])]]
            })
            .collect(),
    };
    LinearRgb::new(data, b.width as usize, b.height as usize).ok()
}

impl Reference {
    /// `None` for images smaller than 8×8 (the metric needs a few pixels to work with).
    pub fn new(b: &Bitmap) -> Option<Self> {
        if b.width < 8 || b.height < 8 {
            return None;
        }
        let views =
            if b.has_alpha() { vec![linear(b, Some(0))?, linear(b, Some(255))?] } else { vec![linear(b, None)?] };
        Some(Self { width: b.width, height: b.height, views })
    }

    fn transparent(&self) -> bool {
        self.views.len() == 2
    }

    /// Lowest score over the views; `None` if it can't be computed.
    pub fn score(&self, distorted: &Bitmap) -> Option<f64> {
        if distorted.width != self.width || distorted.height != self.height {
            return None;
        }
        let backgrounds: &[Option<u8>] = if self.transparent() { &[Some(0), Some(255)] } else { &[None] };
        let mut worst = f64::INFINITY;
        for (view, bg) in self.views.iter().zip(backgrounds) {
            let d = linear(distorted, *bg)?;
            let s = compute_frame_ssimulacra2(view.clone(), d).ok()?;
            worst = worst.min(s);
        }
        Some(worst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Bitmap {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) % 256) as u8, 255]);
            }
        }
        Bitmap::new(w, h, px)
    }

    #[test]
    fn identical_scores_100_and_noise_lowers_it() {
        let a = gradient(64, 64);
        let r = Reference::new(&a).unwrap();
        assert!(r.score(&a).unwrap() > 99.0);
        let mut b = a.clone();
        for (i, v) in b.rgba.iter_mut().enumerate() {
            if i % 4 != 3 && i % 7 == 0 {
                *v = v.saturating_add(40);
            }
        }
        assert!(r.score(&b).unwrap() < 90.0);
        assert!(Reference::new(&gradient(4, 4)).is_none());
    }
}
