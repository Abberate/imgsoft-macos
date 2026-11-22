//! Finds, per image, the lowest encoder quality whose result still reaches the perceptual target:
//! "as small as possible, but never visibly worse than asked".

use std::sync::atomic::{AtomicBool, Ordering};

use crate::Error;
use crate::bitmap::Bitmap;
use crate::decode::decode_output;
use crate::encode::{self, Codec};
use crate::metric::Reference;
use crate::settings::Effort;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub codec: Codec,
    pub bytes: Vec<u8>,
    /// Encoder quality used (lossy codecs).
    pub quality: Option<f32>,
    /// Measured SSIMULACRA2 score (`None` when it could not be measured).
    pub score: Option<f64>,
    /// The target was reached (or assumed, when unmeasurable).
    pub meets_target: bool,
}

pub struct Ctx<'a> {
    pub bitmap: &'a Bitmap,
    pub reference: Option<&'a Reference>,
    pub target: f32,
    pub effort: Effort,
    pub cancel: &'a AtomicBool,
}

impl Ctx<'_> {
    fn check(&self) -> Result<(), Error> {
        if self.cancel.load(Ordering::Relaxed) { Err(Error::cancelled()) } else { Ok(()) }
    }

    fn measure(&self, codec: Codec, bytes: &[u8]) -> Option<f64> {
        let reference = self.reference?;
        let decoded = decode_output(codec, bytes)?;
        reference.score(&decoded)
    }
}

const MIN_Q: f32 = 20.0;
const MAX_Q: f32 = 100.0;
const MAX_STEPS: usize = 8;
/// A passing probe this close to the target is as good as it gets.
const CLOSE_ENOUGH: f64 = 0.4;

/// Starting point of the search: typical quality for this target (calibrated on photos).
pub(crate) fn initial_quality(codec: Codec, target: f32) -> f32 {
    let q = match codec {
        Codec::Jpeg => 88.0 + (target - 85.0) * 1.2,
        Codec::Webp => 86.0 + (target - 85.0) * 1.6,
        _ => 84.0 + (target - 85.0) * 1.5,
    };
    q.round().clamp(MIN_Q, MAX_Q)
}

/// Typical score gained per quality step, used before two probes give a real slope.
fn default_slope(codec: Codec) -> f64 {
    match codec {
        Codec::Avif => 0.6,
        _ => 0.45,
    }
}

fn encode_lossy(ctx: &Ctx<'_>, codec: Codec, q: f32, full_chroma: bool) -> Result<Vec<u8>, Error> {
    match codec {
        Codec::Jpeg => encode::jpeg(ctx.bitmap, q, full_chroma),
        Codec::Webp => encode::webp(ctx.bitmap, q, ctx.effort),
        Codec::Avif => encode::avif(ctx.bitmap, q, ctx.effort),
        _ => unreachable!("not a lossy codec"),
    }
}

/// Best candidate for one codec. `limit` is the size to beat (another codec's result):
/// once a codec provably can't go under it, it is abandoned (`Ok(None)`).
pub fn best_for(ctx: &Ctx<'_>, codec: Codec, limit: Option<usize>) -> Result<Option<Candidate>, Error> {
    match codec {
        Codec::Jpeg => {
            // 4:2:0 is smaller when the target allows it; very high targets need full chroma.
            if ctx.target >= 88.0 {
                return search(ctx, Codec::Jpeg, true, limit);
            }
            match search(ctx, Codec::Jpeg, false, limit)? {
                Some(c) if !c.meets_target => Ok(search(ctx, Codec::Jpeg, true, limit)?.or(Some(c))),
                other => Ok(other),
            }
        }
        Codec::Avif => search(ctx, codec, false, limit),
        Codec::Webp => match search(ctx, codec, false, limit)? {
            // Lossy WebP is always 4:2:0; when that can't reach the target, go lossless.
            Some(c) if !c.meets_target => {
                let l = lossless(ctx, Codec::WebpLossless)?;
                Ok(Some(if l.bytes.len() < c.bytes.len() * 3 { l } else { c }))
            }
            other => Ok(other),
        },
        Codec::WebpLossless | Codec::Png => lossless(ctx, codec).map(Some),
        Codec::PngQuantized => png_best(ctx).map(Some),
    }
}

fn lossless(ctx: &Ctx<'_>, codec: Codec) -> Result<Candidate, Error> {
    ctx.check()?;
    let bytes = match codec {
        Codec::WebpLossless => encode::webp_lossless(ctx.bitmap, ctx.effort)?,
        _ => encode::png_lossless(ctx.bitmap, ctx.effort)?,
    };
    Ok(Candidate { codec, bytes, quality: None, score: Some(100.0), meets_target: true })
}

/// PNG: an 8-bit palette when it looks good enough (often 3–5× smaller), else lossless.
fn png_best(ctx: &Ctx<'_>) -> Result<Candidate, Error> {
    let lossless_png = lossless(ctx, Codec::Png)?;
    let Some(reference) = ctx.reference else { return Ok(lossless_png) };
    let mut best: Option<(encode::Quantized, f64)> = None;
    for colors in [256, 128, 64] {
        ctx.check()?;
        let Ok(q) = encode::quantize(ctx.bitmap, colors, ctx.effort) else { break };
        match reference.score(&q.preview) {
            Some(s) if s >= ctx.target as f64 => best = Some((q, s)),
            _ => break,
        }
    }
    if let Some((q, score)) = best {
        let bytes = encode::png_indexed(&q, ctx.effort)?;
        if bytes.len() < lossless_png.bytes.len() {
            return Ok(Candidate {
                codec: Codec::PngQuantized,
                bytes,
                quality: None,
                score: Some(score),
                meets_target: true,
            });
        }
    }
    Ok(lossless_png)
}

struct Probe {
    q: f32,
    bytes: Vec<u8>,
    score: f64,
}

/// Score change per quality unit, from the two most recent probes (or a typical value).
fn slope(codec: Codec, history: &[(f32, f64)]) -> f64 {
    if let [.., (qa, sa), (qb, sb)] = history
        && (qa - qb).abs() >= 1.0
    {
        let s = (sa - sb) / (qa - qb) as f64;
        if s > 0.05 {
            return s;
        }
    }
    default_slope(codec)
}

fn search(ctx: &Ctx<'_>, codec: Codec, full_chroma: bool, limit: Option<usize>) -> Result<Option<Candidate>, Error> {
    let target = ctx.target as f64;
    let mut q = initial_quality(codec, ctx.target);
    if full_chroma && codec == Codec::Jpeg {
        q = (q - 4.0).max(MIN_Q);
    }

    // Nothing to measure against: trust the calibrated starting point.
    if ctx.reference.is_none() || (codec == Codec::Avif && !avif_measurable()) {
        ctx.check()?;
        let bytes = encode_lossy(ctx, codec, q, full_chroma)?;
        return Ok(Some(Candidate { codec, bytes, quality: Some(q), score: None, meets_target: true }));
    }

    let tolerance = ctx.effort.search_tolerance();
    let debug = std::env::var_os("IMGSOFT_DEBUG").is_some();
    let mut history: Vec<(f32, f64)> = Vec::new();
    let mut pass: Option<Probe> = None; // lowest quality that reaches the target
    let mut fail: Option<Probe> = None; // highest quality that does not
    for _ in 0..MAX_STEPS {
        ctx.check()?;
        let t0 = std::time::Instant::now();
        let bytes = encode_lossy(ctx, codec, q, full_chroma)?;
        let t1 = t0.elapsed();
        let measured = ctx.measure(codec, &bytes);
        if debug {
            eprintln!(
                "  [{}×{}] {codec:?} q{q} → {} o, score {:.2} (encodage {} ms, mesure {} ms)",
                ctx.bitmap.width,
                ctx.bitmap.height,
                bytes.len(),
                measured.unwrap_or(-1.0),
                t1.as_millis(),
                (t0.elapsed() - t1).as_millis()
            );
        }
        let Some(score) = measured else {
            // Could not decode our own output: keep it, unmeasured.
            return Ok(Some(Candidate { codec, bytes, quality: Some(q), score: None, meets_target: true }));
        };
        history.push((q, score));
        let probe = Probe { q, bytes, score };
        if score >= target {
            if pass.as_ref().is_none_or(|p| q < p.q) {
                pass = Some(probe);
            }
        } else {
            // Reaching the target needs a higher quality, hence a bigger file: this codec loses.
            if limit.is_some_and(|l| probe.bytes.len() >= l) {
                return Ok(None);
            }
            if fail.as_ref().is_none_or(|f| q > f.q) {
                fail = Some(probe);
            }
        }

        let slope = slope(codec, &history);
        q = match (&pass, &fail) {
            (Some(p), _) if p.score - target < CLOSE_ENOUGH => break,
            (Some(p), Some(f)) => {
                if p.q - f.q <= tolerance {
                    break;
                }
                // Interpolate inside the bracket, then keep a step of at least 1.
                let t = ((target - f.score) / (p.score - f.score).max(1e-6)).clamp(0.15, 0.85) as f32;
                (f.q + t * (p.q - f.q)).round().clamp(f.q + 1.0, p.q - 1.0)
            }
            (Some(p), None) => {
                if p.q <= MIN_Q {
                    break;
                }
                let step = ((p.score - target) / slope * 0.9) as f32;
                (p.q - step.clamp(2.0, 30.0)).round().max(MIN_Q)
            }
            (None, Some(f)) => {
                if f.q >= MAX_Q {
                    break;
                }
                let step = ((target - f.score) / slope * 1.1) as f32;
                (f.q + step.clamp(2.0, 30.0)).round().min(MAX_Q)
            }
            (None, None) => unreachable!(),
        };
    }

    let best = match (pass, fail) {
        (Some(p), _) => {
            Candidate { codec, bytes: p.bytes, quality: Some(p.q), score: Some(p.score), meets_target: true }
        }
        (None, Some(f)) => {
            Candidate { codec, bytes: f.bytes, quality: Some(f.q), score: Some(f.score), meets_target: false }
        }
        (None, None) => unreachable!(),
    };
    Ok(if limit.is_some_and(|l| best.bytes.len() >= l) { None } else { Some(best) })
}

/// AVIF outputs can only be checked where an AVIF decoder exists (ImageIO, macOS 13+).
pub fn avif_measurable() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        let probe = Bitmap::new(16, 16, [90u8, 140, 200, 255].repeat(256));
        encode::avif(&probe, 80.0, Effort::Fast)
            .ok()
            .and_then(|b| decode_output(Codec::Avif, &b))
            .is_some_and(|d| d.width == 16 && d.height == 16)
    })
}

/// Runs every codec (most promising first) and keeps the smallest result that reaches the
/// target; if none does, the closest one.
pub fn best_of(ctx: &Ctx<'_>, codecs: &[Codec]) -> Result<Option<Candidate>, Error> {
    let mut winner: Option<Candidate> = None;
    let mut fallback: Option<Candidate> = None;
    for &codec in codecs {
        let limit = winner.as_ref().map(|w| w.bytes.len());
        let Some(c) = best_for(ctx, codec, limit)? else { continue };
        if c.meets_target {
            if winner.as_ref().is_none_or(|w| c.bytes.len() < w.bytes.len()) {
                winner = Some(c);
            }
        } else if fallback.as_ref().is_none_or(|f| {
            c.score.unwrap_or(0.0) > f.score.unwrap_or(0.0) || (c.score == f.score && c.bytes.len() < f.bytes.len())
        }) {
            fallback = Some(c);
        }
    }
    Ok(winner.or(fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo_like(w: u32, h: u32) -> Bitmap {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        let mut s = 7u32;
        for y in 0..h {
            for x in 0..w {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = (s >> 28) as i32 - 8;
                let r = ((x * 255 / w) as i32 + n).clamp(0, 255) as u8;
                let g = ((y * 255 / h) as i32 + n).clamp(0, 255) as u8;
                let b = (((x ^ y) & 0xFF) as i32 / 2 + 64 + n).clamp(0, 255) as u8;
                px.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Bitmap::new(w, h, px)
    }

    #[test]
    fn jpeg_search_reaches_the_target_with_a_lower_quality_than_100() {
        let img = photo_like(128, 96);
        let reference = Reference::new(&img).unwrap();
        let cancel = AtomicBool::new(false);
        let ctx =
            Ctx { bitmap: &img, reference: Some(&reference), target: 80.0, effort: Effort::Balanced, cancel: &cancel };
        let c = best_for(&ctx, Codec::Jpeg, None).unwrap().unwrap();
        assert!(c.meets_target);
        assert!(c.score.unwrap() >= 80.0);
        assert!(c.quality.unwrap() < 100.0);
        // A size limit far below what JPEG can do makes it give up.
        assert!(best_for(&ctx, Codec::Jpeg, Some(10)).unwrap().is_none());
    }

    #[test]
    fn cancellation_stops_the_search() {
        let img = photo_like(64, 64);
        let reference = Reference::new(&img).unwrap();
        let cancel = AtomicBool::new(true);
        let ctx =
            Ctx { bitmap: &img, reference: Some(&reference), target: 85.0, effort: Effort::Fast, cancel: &cancel };
        assert!(best_for(&ctx, Codec::Webp, None).unwrap_err().is_cancelled());
    }
}
