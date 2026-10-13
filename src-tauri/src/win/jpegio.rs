//! JPEG encoding on Windows with `jpeg-encoder` (SIMD accelerated).
//!
//! Same public surface as the macOS `jpegio.rs`.

use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

/// `bgra` is tightly packed 32-bit BGRA (alpha ignored), `quality` 1..=100.
pub fn encode_bgra(bgra: Vec<u8>, w: usize, h: usize, quality: u8) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || w > u16::MAX as usize || h > u16::MAX as usize || bgra.len() < w * h * 4 {
        return None;
    }
    let mut out = Vec::with_capacity(w * h / 4);
    let mut enc = Encoder::new(&mut out, quality.clamp(1, 100));
    enc.set_sampling_factor(SamplingFactor::F_2_2);
    enc.encode(&bgra, w as u16, h as u16, ColorType::Bgra).ok()?;
    Some(out)
}
