//! Native microphone levelling: decode the recorded mic file, mix it to mono, lift
//! quiet recordings to a healthy speech level (never turning anything down) and write a
//! 16-bit stereo WAV next to the project. The editor plays that file with a plain audio
//! element, so nothing heavy runs on the web view's main thread.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use symphonia::core::audio::sample::Sample;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

const OUT_NAME: &str = "mic-leveled-v2.wav";

/// Returns (mono samples, sample rate).
fn decode_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let r = decode_mono_symphonia(path);
    // Chromium's MediaRecorder writes AAC headers symphonia rejects; Media Foundation reads them
    #[cfg(windows)]
    if r.is_err() {
        return decode_mono_mf(path);
    }
    r
}

#[cfg(windows)]
fn decode_mono_mf(path: &Path) -> Result<(Vec<f32>, u32), String> {
    crate::win::com_init();
    let mut reader = crate::win::mf::AudioReader::open(&path.to_string_lossy())?;
    let ch = reader.channels.max(1) as usize;
    let mut mono = Vec::new();
    while let Some((_, pcm)) = reader.next() {
        for frame in pcm.chunks(ch) {
            mono.push(frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / ch as f32);
        }
    }
    if mono.is_empty() {
        return Err("no audio decoded".into());
    }
    Ok((mono, reader.rate))
}

fn decode_mono_symphonia(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| e.to_string())?;
    let track = format.default_track(TrackType::Audio).ok_or("no audio track")?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|c| c.audio())
        .ok_or("not an audio codec")?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| e.to_string())?;

    let mut mono: Vec<f32> = Vec::new();
    let mut tmp: Vec<f32> = Vec::new();
    let mut rate = 48_000u32;
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(_) => break,
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = buf.spec();
                rate = spec.rate();
                let ch = spec.channels().count().max(1);
                tmp.resize(buf.samples_interleaved(), f32::MID);
                buf.copy_to_slice_interleaved(&mut tmp);
                for frame in tmp.chunks(ch) {
                    mono.push(frame.iter().sum::<f32>() / ch as f32);
                }
            }
            Err(Error::DecodeError(_)) => {}
            Err(_) => break,
        }
    }
    if mono.is_empty() {
        return Err("no audio decoded".into());
    }
    Ok((mono, rate))
}

/// Gain that brings the loud parts to about -20 dBFS, capped so peaks stay under 0.9.
fn auto_gain(samples: &[f32], rate: u32) -> f32 {
    let win = ((rate as f32 * 0.05) as usize).max(1);
    let mut rms: Vec<f32> = Vec::with_capacity(samples.len() / win + 1);
    let mut peak = 0f32;
    for w in samples.chunks_exact(win) {
        let mut sum = 0f32;
        for &x in w {
            sum += x * x;
            peak = peak.max(x.abs());
        }
        rms.push((sum / win as f32).sqrt());
    }
    if rms.is_empty() || peak < 1e-5 {
        return 1.0;
    }
    rms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let top = &rms[rms.len() * 4 / 5..];
    let speech = top.iter().sum::<f32>() / top.len().max(1) as f32;
    (0.1 / speech.max(1e-5)).min(0.9 / peak).clamp(1.0, 60.0)
}

fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

fn write_wav(path: &Path, samples: &[f32], rate: u32) -> Result<(), String> {
    let data_len = (samples.len() * 4) as u32; // stereo, 16-bit
    let mut f = std::io::BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data_len).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM
    h.extend_from_slice(&2u16.to_le_bytes()); // channels
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * 4).to_le_bytes());
    h.extend_from_slice(&4u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data_len.to_le_bytes());
    f.write_all(&h).map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity(samples.len() * 4);
    for &x in samples {
        let v = (x.clamp(-1.0, 1.0) * 32767.0) as i16;
        let b = v.to_le_bytes();
        buf.extend_from_slice(&b);
        buf.extend_from_slice(&b);
    }
    f.write_all(&buf).map_err(|e| e.to_string())?;
    f.flush().map_err(|e| e.to_string())
}

pub fn level(dir: &Path, rel: &str) -> Result<String, String> {
    let src = dir.join(rel);
    let out: PathBuf = dir.join(OUT_NAME);
    // reuse the cached file when it is newer than the recording
    if let (Ok(a), Ok(b)) = (std::fs::metadata(&src).and_then(|m| m.modified()), std::fs::metadata(&out).and_then(|m| m.modified())) {
        if b >= a {
            return Ok(OUT_NAME.into());
        }
    }
    let (mono, rate) = decode_mono(&src)?;
    let gain = auto_gain(&mono, rate);
    // x2: headroom so the 100% master position is louder than 50%; the editor scales it back
    let leveled: Vec<f32> = mono.iter().map(|&x| soft_limit(x * gain)).collect();
    write_wav(&out, &leveled, rate)?;
    Ok(OUT_NAME.into())
}

/// Runs on a blocking worker thread so the UI never waits for it.
#[tauri::command]
pub async fn level_mic(dir: String, file: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || level(Path::new(&dir), &file))
        .await
        .map_err(|e| e.to_string())?
}
