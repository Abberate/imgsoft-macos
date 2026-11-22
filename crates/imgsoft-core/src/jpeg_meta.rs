//! Lossless JPEG clean-up: drops EXIF (GPS, camera serials), XMP, IPTC, comments, thumbnails and
//! the extra pictures phones append after the image (depth maps, HDR gain maps).
//! Keeps what changes how the picture looks: JFIF, ICC profile, Adobe colour transform.

/// `None` if the file is not a JPEG we can safely rewrite.
pub fn strip(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    let mut i = 2;
    loop {
        // Skip fill bytes.
        while i < bytes.len() && bytes[i] == 0xFF && bytes.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        if i + 4 > bytes.len() || bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            return None;
        }
        let segment = &bytes[i..i + 2 + len];
        let payload = &segment[4..];
        let keep = match marker {
            0xE0 => payload.starts_with(b"JFIF\0"),
            0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
            0xEE => payload.starts_with(b"Adobe"),
            0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,
            _ => true,
        };
        if keep {
            out.extend_from_slice(segment);
        }
        i += 2 + len;
        if marker == 0xDA {
            break;
        }
    }
    // Entropy-coded data and later scans, up to the first real end-of-image marker
    // (inside scans every 0xFF byte is followed by 0x00 or a restart marker).
    let rest = &bytes[i..];
    let end = rest.windows(2).position(|w| w == [0xFF, 0xD9])? + 2;
    out.extend_from_slice(&rest[..end]);
    Some(out)
}

/// EXIF orientation other than "normal": the stripped file would display rotated.
pub fn needs_rotation(bytes: &[u8]) -> bool {
    use image::ImageDecoder;
    image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(bytes))
        .ok()
        .and_then(|mut d| d.orientation().ok())
        .is_some_and(|o| o != image::metadata::Orientation::NoTransforms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_exif_and_trailing_data_keeps_icc() {
        let img = crate::bitmap::Bitmap::new(16, 16, [200u8, 100, 50, 255].repeat(256));
        let jpeg = crate::encode::jpeg(&img, 80.0, false).unwrap();
        // Insert an EXIF segment and an ICC segment after SOI, and junk after EOI.
        let mut with_meta = vec![0xFF, 0xD8];
        let exif = b"Exif\0\0GPS-SECRET";
        with_meta.extend_from_slice(&[0xFF, 0xE1]);
        with_meta.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        with_meta.extend_from_slice(exif);
        let icc = b"ICC_PROFILE\0\x01\x01data";
        with_meta.extend_from_slice(&[0xFF, 0xE2]);
        with_meta.extend_from_slice(&((icc.len() + 2) as u16).to_be_bytes());
        with_meta.extend_from_slice(icc);
        with_meta.extend_from_slice(&jpeg[2..]);
        with_meta.extend_from_slice(b"TRAILING-DEPTH-MAP");

        let out = strip(&with_meta).unwrap();
        let has = |hay: &[u8], n: &[u8]| hay.windows(n.len()).any(|w| w == n);
        assert!(!has(&out, b"GPS-SECRET"));
        assert!(!has(&out, b"TRAILING"));
        assert!(has(&out, b"ICC_PROFILE"));
        assert!(out.ends_with(&[0xFF, 0xD9]));
        assert!(image::load_from_memory(&out).is_ok());
    }
}
