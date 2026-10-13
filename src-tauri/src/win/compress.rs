//! HandBrake-style video compression on Windows with Media Foundation (hardware
//! encoders when available). Reads any video Windows can decode, re-encodes it with
//! a target quality, codec, resolution and frame rate, and re-encodes the audio.
//!
//! Same public surface as the macOS `compress.rs`.

use crate::win::mf;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_s: f64,
    pub video_kbps: f64,
    pub has_audio: bool,
    pub file_size: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressOpts {
    pub input: String,
    pub output: String,
    /// "h264" | "hevc"
    pub codec: String,
    /// 0..=100, higher = better quality / bigger file.
    pub quality: f64,
    /// Downscale to this height (never upscales).
    pub height: Option<u32>,
    /// Limit the frame rate (never increases it).
    pub fps: Option<f64>,
    /// "keep" | "aac160" | "aac128" | "aac96" | "aac64" | "none"
    pub audio: String,
    #[serde(default)]
    pub optimize_for_web: bool,
}

pub fn probe(path: &str) -> Result<Probe, String> {
    let info = mf::probe(path)?;
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    // containers don't always state the video bit rate: estimate it from the file size
    let video_kbps = if info.video_bps > 0.0 {
        info.video_bps / 1000.0
    } else if info.duration_s > 0.0 {
        file_size as f64 * 8.0 / info.duration_s / 1000.0 * 0.95
    } else {
        0.0
    };
    Ok(Probe {
        width: info.width,
        height: info.height,
        fps: info.fps,
        duration_s: info.duration_s,
        video_kbps,
        has_audio: info.has_audio,
        file_size,
    })
}

fn even(v: f64) -> u32 {
    let n = v.round().max(2.0) as u32;
    n - (n % 2)
}

/// Target video bitrate from the quality slider (bits per pixel per frame).
pub fn target_bitrate(width: f64, height: f64, fps: f64, quality: f64, hevc: bool) -> f64 {
    let q = (quality / 100.0).clamp(0.0, 1.0);
    let bpp = 0.015 + q * q * 0.2;
    let bpp = if hevc { bpp * 0.62 } else { bpp };
    (width * height * fps * bpp).clamp(250_000.0, 100_000_000.0)
}

pub fn compress(opts: &CompressOpts, mut progress: impl FnMut(f64)) -> Result<(), String> {
    CANCEL.store(false, Ordering::SeqCst);
    crate::win::com_init();
    let info = probe(&opts.input)?;

    // ---- output geometry / rate
    let (sw, sh) = (info.width as f64, info.height as f64);
    let scale = match opts.height {
        Some(h) if (h as f64) < sh => h as f64 / sh,
        _ => 1.0,
    };
    let (ow, oh) = (even(sw * scale), even(sh * scale));
    let src_fps = if info.fps > 1.0 { info.fps } else { 30.0 };
    let out_fps = opts.fps.map(|f| f.min(src_fps)).unwrap_or(src_fps);
    let hevc = opts.codec == "hevc";
    let mut bitrate = target_bitrate(ow as f64, oh as f64, out_fps, opts.quality, hevc);
    if info.video_kbps > 0.0 {
        bitrate = bitrate.min(info.video_kbps * 1000.0 * 0.98);
    }

    // ---- readers (one per track, so each can be pumped at its own pace)
    let mut vr = mf::VideoReader::open(&opts.input, Some(ow))?;
    let mut ar = if opts.audio != "none" && info.has_audio { mf::AudioReader::open(&opts.input).ok() } else { None };

    // ---- writer
    let kbps: u32 = if opts.audio == "keep" {
        192
    } else {
        opts.audio.strip_prefix("aac").and_then(|s| s.parse().ok()).unwrap_or(128)
    };
    let aspec = ar.as_ref().map(|a| mf::AudioSpec { rate: a.rate, channels: a.channels, kbps });
    let vspec = mf::VideoSpec {
        width: ow,
        height: oh,
        fps: out_fps,
        bitrate: bitrate as u32,
        hevc,
        gop: (out_fps * 2.0).round() as u32,
    };
    let mut writer = mf::Writer::create(&opts.output, &vspec, aspec.as_ref(), opts.optimize_for_web)?;
    // the writer only takes 44.1/48 kHz mono/stereo; anything else is dropped
    if let (Some(a), Some((rate, ch))) = (&ar, writer.audio_format()) {
        if a.rate != rate || a.channels != ch {
            ar = None;
        }
    }

    let cancelled = |writer: mf::Writer| -> Result<(), String> {
        drop(writer);
        let _ = std::fs::remove_file(&opts.output);
        Err("cancelled".into())
    };

    // ---- pump: video drives, audio is kept slightly ahead so the file stays interleaved
    let total = info.duration_s.max(0.001);
    let frame_dt = 1.0 / out_fps;
    let mut next_keep = 0.0f64;
    let mut last_report = 0.0;
    let mut audio_t = 0.0f64;
    let mut pump_audio = |writer: &mut mf::Writer, ar: &mut Option<mf::AudioReader>, until: f64| -> Result<(), String> {
        while let Some(a) = ar.as_mut() {
            if audio_t > until {
                break;
            }
            match a.next() {
                Some((t, pcm)) => {
                    writer.write_audio(&pcm, t)?;
                    audio_t = t;
                }
                None => *ar = None,
            }
        }
        Ok(())
    };
    while let Some(f) = vr.next() {
        if CANCEL.load(Ordering::SeqCst) {
            return cancelled(writer);
        }
        if f.pts + 1e-4 >= next_keep || out_fps >= src_fps - 0.01 {
            let px = mf::resize_bgra(&f.data, f.w as usize, f.h as usize, ow as usize, oh as usize);
            writer.write_video(&px, f.pts)?;
            next_keep = f.pts + frame_dt * 0.98;
        }
        pump_audio(&mut writer, &mut ar, f.pts + 0.5)?;
        if f.pts - last_report > 0.25 {
            last_report = f.pts;
            progress((f.pts / total).clamp(0.0, 1.0));
        }
    }
    pump_audio(&mut writer, &mut ar, f64::INFINITY)?;

    writer.finish()?;
    progress(1.0);
    Ok(())
}
