//! 8-bit sRGB RGBA picture with straight (non-premultiplied) alpha: the engine's working format.

#[derive(Clone)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, R G B A.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bitmap({}×{})", self.width, self.height)
    }
}

impl Bitmap {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        assert_eq!(rgba.len(), width as usize * height as usize * 4, "RGBA buffer size");
        Self { width, height, rgba }
    }

    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn has_alpha(&self) -> bool {
        self.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 255)
    }

    pub fn is_grayscale(&self) -> bool {
        self.rgba.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2])
    }

    pub fn to_rgb(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixel_count() * 3);
        for p in self.rgba.as_chunks::<4>().0 {
            out.extend_from_slice(&p[..3]);
        }
        out
    }

    pub fn to_gray(&self) -> Vec<u8> {
        self.rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect()
    }

    pub fn as_rgba8(&self) -> &[rgb::RGBA8] {
        use rgb::FromSlice;
        self.rgba.as_rgba()
    }

    /// Fully transparent pixels get a neutral colour: invisible, but it no longer costs bytes.
    pub fn clean_transparent(&mut self) {
        for p in self.rgba.as_chunks_mut::<4>().0 {
            if p[3] == 0 {
                p[0] = 0;
                p[1] = 0;
                p[2] = 0;
            }
        }
    }

    /// Rough content class: flat graphics (logos, screenshots, UI) compress best losslessly.
    pub fn looks_like_graphic(&self) -> bool {
        let n = self.pixel_count();
        if n == 0 {
            return false;
        }
        // Few distinct colours → palette/lossless territory.
        let mut seen = std::collections::HashSet::with_capacity(512);
        let mut few_colors = true;
        // Sample every pixel for small images, a grid for big ones (keeps this < 5 ms).
        let step = (n / 400_000).max(1);
        for p in self.rgba.as_chunks::<4>().0.iter().step_by(step) {
            seen.insert(u32::from_le_bytes([p[0], p[1], p[2], p[3]]));
            if seen.len() > 256 {
                few_colors = false;
                break;
            }
        }
        if few_colors {
            return true;
        }
        // Large flat areas: share of pixels identical to their left neighbour.
        let w = self.width as usize;
        let mut same = 0usize;
        let mut total = 0usize;
        for row in self.rgba.chunks_exact(w * 4).step_by(step.max(1)) {
            for x in 1..w {
                total += 1;
                if row[x * 4..x * 4 + 4] == row[x * 4 - 4..x * 4] {
                    same += 1;
                }
            }
        }
        total > 0 && same as f64 / total as f64 > 0.55
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_flat_and_noisy_content() {
        let flat = Bitmap::new(64, 64, [10u8, 20, 30, 255].repeat(64 * 64));
        assert!(flat.looks_like_graphic());
        assert!(!flat.has_alpha());

        let mut noisy = Vec::with_capacity(64 * 64 * 4);
        let mut s = 12345u32;
        for _ in 0..64 * 64 {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let b = s.to_le_bytes();
            noisy.extend_from_slice(&[b[0], b[1], b[2], 255]);
        }
        assert!(!Bitmap::new(64, 64, noisy).looks_like_graphic());
    }
}
