//! Preview proxy: the recorded screen / camera videos are decoded once, natively, and
//! stored as small JPEG strips that the editor reads frame by frame over the `sc://`
//! scheme. The web view then never plays or seeks a <video> for the preview, which is
//! what used to deadlock WebKit's media player.
//!
//! Files per project (all in the project folder):
//!   proxy-<kind>.sci  concatenated JPEG frames
//!   proxy-<kind>.idx  (n+1) little-endian u64 byte offsets
//!   proxy.json        {"version", "screen":{"n","fps","w","h"}, "camera":{...}}

#![allow(deprecated)]

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2::AnyThread;
#[cfg(target_os = "macos")]
use objc2_av_foundation::{AVAssetReader, AVAssetReaderTrackOutput, AVURLAsset};
#[cfg(target_os = "macos")]
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::collections::BTreeMap;
use std::fs::File;
#[cfg(target_os = "macos")]
use std::io::{BufWriter, Write};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use crate::win::strip::build_strip;

pub const FPS: f64 = 30.0;
const VERSION: u32 = 5;

#[derive(Serialize, Clone, Debug, Default)]
pub struct StripMeta {
    pub n: usize,
    pub fps: f64,
    pub w: u32,
    pub h: u32,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ProxyMeta {
    pub version: u32,
    pub screen: Option<StripMeta>,
    pub camera: Option<StripMeta>,
}

#[cfg(target_os = "macos")]
/// Presentation time in seconds with 32-bit timestamp wrap-around undone.
///
/// Browser-recorded fragmented MP4s sometimes carry negative composition offsets stored
/// as unsigned numbers, which AVFoundation reads as 2^32 ticks too late (about 7,158,278 s
/// at a 600 Hz timescale). Everything after that point then looked "in the far future" and
/// the facecam froze. Fold such values back by whole multiples of 2^32 ticks.
pub fn unwrapped_seconds(t: objc2_core_media::CMTime) -> f64 {
    let (v, ts) = (t.value, t.timescale as f64);
    if ts <= 0.0 {
        return unsafe { t.seconds() };
    }
    const WRAP: i64 = 1 << 32;
    let v = if v >= WRAP / 2 { v - WRAP * ((v + WRAP / 2) / WRAP) } else { v };
    v as f64 / ts
}

#[cfg(target_os = "macos")]
/// The raw (still wrapped) time to ask AVAssetReader for when a sample was seen at `t`.
pub fn raw_seconds(t: objc2_core_media::CMTime) -> f64 {
    unsafe { t.seconds() }
}

#[cfg(target_os = "macos")]
fn obj<T: objc2::Message>(r: Retained<T>) -> Retained<AnyObject> {
    unsafe { Retained::cast_unchecked(r) }
}
#[cfg(target_os = "macos")]
fn num_i(v: i64) -> Retained<AnyObject> {
    obj(NSNumber::new_i64(v))
}
#[cfg(target_os = "macos")]
fn dict(pairs: Vec<(&str, Retained<AnyObject>)>) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let vals: Vec<Retained<AnyObject>> = pairs.into_iter().map(|(_, v)| v).collect();
    NSDictionary::from_retained_objects(&key_refs, &vals)
}

#[cfg(target_os = "macos")]
fn encode_jpeg(bgra: Vec<u8>, w: u32, h: u32, quality: u8) -> Vec<u8> {
    crate::jpegio::encode_bgra(bgra, w as usize, h as usize, quality).unwrap_or_default()
}

#[cfg(target_os = "macos")]
/// Decode `src` and write the strip. `target_w` is the proxy width in pixels.
fn build_strip(
    src: &Path,
    out_sci: &Path,
    out_idx: &Path,
    target_w: u32,
    quality: u8,
    progress: &mut dyn FnMut(f64),
) -> Result<StripMeta, String> {
    unsafe {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&src.to_string_lossy()));
        let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
        let vtype = NSString::from_str("vide");
        let track = asset.tracksWithMediaType(&vtype).firstObject().ok_or("no video track")?;
        let size = track.naturalSize();
        let duration = asset.duration().seconds();
        if size.width < 2.0 || size.height < 2.0 {
            return Err("empty video".into());
        }
        let w = (target_w as f64).min(size.width).round() as u32 & !1;
        let h = ((w as f64 * size.height / size.width).round() as u32) & !1;

        let settings = dict(vec![
            ("PixelFormatType", num_i(1111970369)), // 32BGRA
            ("Width", num_i(w as i64)),
            ("Height", num_i(h as i64)),
        ]);
        // (re)open a reader, optionally starting part-way (used to resume after an early end)
        let open_reader = |start_s: f64| -> Result<(Retained<AVAssetReader>, Retained<AVAssetReaderTrackOutput>), String> {
            let reader = AVAssetReader::initWithAsset_error(AVAssetReader::alloc(), &asset)
                .map_err(|e| e.localizedDescription().to_string())?;
            let out = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, Some(&settings));
            reader.addOutput(&out);
            if start_s > 0.0 {
                let span = if duration.is_finite() && duration > start_s { duration - start_s } else { 86_400.0 };
                reader.setTimeRange(objc2_core_media::CMTimeRange {
                    start: objc2_core_media::CMTime::new((start_s * 1000.0) as i64, 1000),
                    duration: objc2_core_media::CMTime::new(((span + 1.0) * 1000.0) as i64, 1000),
                });
            }
            if !reader.startReading() {
                return Err("could not start reading the video".into());
            }
            Ok((reader, out))
        };
        let (mut reader, mut vout) = open_reader(0.0)?;

        // encoder workers: slot -> jpeg
        let (job_tx, job_rx) = mpsc::sync_channel::<(usize, Vec<u8>)>(8);
        let job_rx = Arc::new(Mutex::new(job_rx));
        let (res_tx, res_rx) = mpsc::channel::<(usize, Vec<u8>)>();
        let workers: Vec<_> = (0..3)
            .map(|_| {
                let rx = job_rx.clone();
                let tx = res_tx.clone();
                std::thread::spawn(move || loop {
                    let job = rx.lock().unwrap().recv();
                    match job {
                        Ok((slot, px)) => {
                            let jpg = encode_jpeg(px, w, h, quality);
                            let _ = tx.send((slot, jpg));
                        }
                        Err(_) => break,
                    }
                })
            })
            .collect();
        drop(res_tx);

        eprintln!("[proxy] {} -> {w}x{h} (source {}x{}, {:.1}s)", src.display(), size.width, size.height, duration);
        let mut sci = BufWriter::new(File::create(out_sci).map_err(|e| e.to_string())?);
        // (start, end) byte range per slot; slots that repeat the previous frame reuse its range
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        let mut pending: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        let mut dups: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        let mut next_write = 0usize;
        let mut cursor = 0u64;
        let mut flush_ready = |pending: &mut BTreeMap<usize, Vec<u8>>,
                               dups: &mut std::collections::BTreeSet<usize>,
                               next_write: &mut usize,
                               ranges: &mut Vec<(u64, u64)>,
                               cursor: &mut u64|
         -> Result<(), String> {
            loop {
                if dups.remove(next_write) && !ranges.is_empty() {
                    let last = *ranges.last().unwrap();
                    ranges.push(last);
                    *next_write += 1;
                } else if let Some(j) = pending.remove(next_write) {
                    sci.write_all(&j).map_err(|e| e.to_string())?;
                    ranges.push((*cursor, *cursor + j.len() as u64));
                    *cursor += j.len() as u64;
                    *next_write += 1;
                } else {
                    break;
                }
            }
            Ok(())
        };

        let step = 1.0 / FPS;
        let mut slot = 0usize; // next slot to emit (slot k is the frame shown at k / FPS)
        let mut shift = 0.0f64; // timestamp discontinuity compensation
        let mut last_raw = 0.0f64;
        let mut last_file_time = 0.0f64;
        let mut resumes = 0;
        'read: loop {
        while let Some(sample) = vout.copyNextSampleBuffer() {
            let raw_pts = unwrapped_seconds(sample.presentation_time_stamp());
            if !raw_pts.is_finite() {
                continue;
            }
            last_raw = raw_pts;
            last_file_time = raw_seconds(sample.presentation_time_stamp());
            let Some(img) = sample.image_buffer() else { continue };
            let mut pts = raw_pts - shift;
            let want_t = slot as f64 * step;
            if pts - want_t > 2.0 {
                // a long timestamp jump (some recorders emit them): don't pad it, just continue
                shift += pts - want_t;
                pts = want_t;
            }
            if want_t > pts + 0.0005 {
                continue; // not yet time for the next slot
            }
            CVPixelBufferLockBaseAddress(&img, CVPixelBufferLockFlags::ReadOnly);
            let (pw, ph) = (CVPixelBufferGetWidth(&img), CVPixelBufferGetHeight(&img));
            let stride = CVPixelBufferGetBytesPerRow(&img);
            let base = CVPixelBufferGetBaseAddress(&img) as *const u8;
            let mut px = vec![0u8; (w * h * 4) as usize];
            if !base.is_null() && pw as u32 == w && ph as u32 == h {
                for row in 0..h as usize {
                    let s = std::slice::from_raw_parts(base.add(row * stride), (w * 4) as usize);
                    px[row * (w * 4) as usize..(row + 1) * (w * 4) as usize].copy_from_slice(s);
                }
            } else if !base.is_null() && pw > 0 && ph > 0 {
                // the reader did not scale: nearest-neighbour fit (slow path, rare)
                for y in 0..h as usize {
                    let sy = (y * ph / h as usize).min(ph - 1);
                    for x in 0..w as usize {
                        let sx = (x * pw / w as usize).min(pw - 1);
                        let s = std::slice::from_raw_parts(base.add(sy * stride + sx * 4), 4);
                        px[(y * w as usize + x) * 4..(y * w as usize + x) * 4 + 4].copy_from_slice(s);
                    }
                }
            }
            CVPixelBufferUnlockBaseAddress(&img, CVPixelBufferLockFlags::ReadOnly);

            // this frame covers every slot up to its timestamp; only the first needs encoding
            let mut first = true;
            while (slot as f64) * step <= pts + 0.0005 {
                if first {
                    job_tx.send((slot, std::mem::take(&mut px))).map_err(|e| e.to_string())?;
                    first = false;
                } else {
                    dups.insert(slot);
                }
                slot += 1;
            }
            while let Ok((sl, jpg)) = res_rx.try_recv() {
                pending.insert(sl, jpg);
            }
            flush_ready(&mut pending, &mut dups, &mut next_write, &mut ranges, &mut cursor)?;
            if duration > 0.0 && duration < 86_400.0 {
                progress((raw_pts / duration).clamp(0.0, 1.0));
            }
        }
        // the reader sometimes ends well before the clip does (damaged / odd timestamps):
        // pick the file up again just past the last frame instead of freezing the strip
        if duration.is_finite() && duration < 86_400.0 && last_raw < duration - 1.5 && resumes < 40 {
            resumes += 1;
            match open_reader(last_file_time + 0.05) {
                Ok((r, o)) => {
                    reader = r;
                    vout = o;
                    continue 'read;
                }
                Err(_) => {}
            }
        }
        break 'read;
        }
        let _ = &reader;
        drop(job_tx);
        for wk in workers {
            let _ = wk.join();
        }
        while let Ok((sl, jpg)) = res_rx.recv() {
            pending.insert(sl, jpg);
        }
        flush_ready(&mut pending, &mut dups, &mut next_write, &mut ranges, &mut cursor)?;
        sci.flush().map_err(|e| e.to_string())?;
        if next_write == 0 || ranges.is_empty() {
            return Err("no frames were decoded".into());
        }

        let mut idx = BufWriter::new(File::create(out_idx).map_err(|e| e.to_string())?);
        for (a, b) in &ranges {
            idx.write_all(&a.to_le_bytes()).map_err(|e| e.to_string())?;
            idx.write_all(&b.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        idx.flush().map_err(|e| e.to_string())?;
        Ok(StripMeta { n: ranges.len(), fps: FPS, w, h })
    }
}

fn meta_path(dir: &Path) -> std::path::PathBuf {
    dir.join("proxy.json")
}

fn newer(a: &Path, than: &Path) -> bool {
    match (std::fs::metadata(a).and_then(|m| m.modified()), std::fs::metadata(than).and_then(|m| m.modified())) {
        (Ok(x), Ok(y)) => x >= y,
        _ => false,
    }
}

/// Build (or reuse) the proxy for a project. `progress` gets 0..1 over both strips.
pub fn ensure(
    dir: &Path,
    screen_rel: &str,
    camera_rel: Option<&str>,
    mut progress: impl FnMut(f64),
) -> Result<ProxyMeta, String> {
    let screen_src = dir.join(screen_rel);
    let cam_src = camera_rel.map(|c| dir.join(c));

    // reuse
    if let Ok(bytes) = std::fs::read(meta_path(dir)) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let fresh = v["version"].as_u64() == Some(VERSION as u64)
                && newer(&meta_path(dir), &screen_src)
                && cam_src.as_ref().map(|c| newer(&meta_path(dir), c)).unwrap_or(true)
                && dir.join("proxy-screen.idx").exists();
            if fresh {
                let strip = |k: &str| -> Option<StripMeta> {
                    let s = v.get(k)?;
                    Some(StripMeta {
                        n: s["n"].as_u64()? as usize,
                        fps: s["fps"].as_f64()?,
                        w: s["w"].as_u64()? as u32,
                        h: s["h"].as_u64()? as u32,
                    })
                };
                return Ok(ProxyMeta { version: VERSION, screen: strip("screen"), camera: strip("camera") });
            }
        }
    }

    let two = cam_src.is_some();
    let screen = build_strip(
        &screen_src,
        &dir.join("proxy-screen.sci"),
        &dir.join("proxy-screen.idx"),
        1280,
        62,
        &mut |p| progress(if two { p * 0.7 } else { p }),
    )?;
    let camera = match &cam_src {
        Some(c) => build_strip(c, &dir.join("proxy-camera.sci"), &dir.join("proxy-camera.idx"), 480, 60, &mut |p| {
            progress(0.7 + p * 0.3)
        })
        .ok(),
        None => None,
    };
    let meta = ProxyMeta { version: VERSION, screen: Some(screen), camera };
    std::fs::write(meta_path(dir), serde_json::to_vec(&meta).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    progress(1.0);
    Ok(meta)
}

// ------------------------------------------------------------------ serving

/// Open strips by "<dir>|<kind>" for the `sc://` scheme.
pub struct Strip {
    file: File,
    ranges: Vec<(u64, u64)>,
}

static STRIPS: Mutex<Option<std::collections::HashMap<String, Arc<Strip>>>> = Mutex::new(None);

pub fn frame_bytes(dir: &str, kind: &str, n: usize) -> Option<Vec<u8>> {
    let key = format!("{dir}|{kind}");
    let strip = {
        let mut g = STRIPS.lock().unwrap();
        let map = g.get_or_insert_with(Default::default);
        if let Some(s) = map.get(&key) {
            s.clone()
        } else {
            let base = Path::new(dir);
            let idx = std::fs::read(base.join(format!("proxy-{kind}.idx"))).ok()?;
            let ranges: Vec<(u64, u64)> = idx
                .chunks_exact(16)
                .map(|c| (u64::from_le_bytes(c[..8].try_into().unwrap()), u64::from_le_bytes(c[8..].try_into().unwrap())))
                .collect();
            let file = File::open(base.join(format!("proxy-{kind}.sci"))).ok()?;
            let s = Arc::new(Strip { file, ranges });
            map.insert(key, s.clone());
            s
        }
    };
    let n = n.min(strip.ranges.len().checked_sub(1)?);
    let (a, b) = strip.ranges[n];
    let mut buf = vec![0u8; (b - a) as usize];
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        strip.file.read_exact_at(&mut buf, a).ok()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            let n = strip.file.seek_read(&mut buf[done..], a + done as u64).ok()?;
            if n == 0 {
                return None;
            }
            done += n;
        }
    }
    Some(buf)
}

/// Forget cached handles for a project (after it is rebuilt).
pub fn forget(dir: &str) {
    if let Some(m) = STRIPS.lock().unwrap().as_mut() {
        m.retain(|k, _| !k.starts_with(&format!("{dir}|")));
    }
}

/// Projects whose proxy is being built right now, with the latest progress. Lets the
/// build that starts when a recording stops be shared with the editor that opens on it.
static BUILDING: Mutex<Option<std::collections::HashMap<String, f64>>> = Mutex::new(None);

fn begin(dir: &str) -> bool {
    let mut g = BUILDING.lock().unwrap();
    let m = g.get_or_insert_with(Default::default);
    if m.contains_key(dir) {
        false
    } else {
        m.insert(dir.to_string(), 0.0);
        true
    }
}

fn set_progress(dir: &str, p: f64) {
    if let Some(m) = BUILDING.lock().unwrap().as_mut() {
        if let Some(v) = m.get_mut(dir) {
            *v = p;
        }
    }
}

fn end(dir: &str) {
    if let Some(m) = BUILDING.lock().unwrap().as_mut() {
        m.remove(dir);
    }
}

fn progress_of(dir: &str) -> Option<f64> {
    BUILDING.lock().unwrap().as_ref().and_then(|m| m.get(dir).copied())
}

fn build_tracked(app: &tauri::AppHandle, dir: &str, screen: &str, camera: Option<&str>) -> Result<ProxyMeta, String> {
    use tauri::Emitter;
    forget(dir);
    let mut last = -1.0f64;
    let r = ensure(Path::new(dir), screen, camera, |p| {
        set_progress(dir, p);
        if p - last >= 0.01 || p >= 1.0 {
            last = p;
            let _ = app.emit("proxy-progress", p);
        }
    });
    forget(dir);
    r
}

/// Kick off the proxy build (and mic levelling) as soon as a recording is saved, so
/// it is usually finished by the time the editor opens.
pub fn start_background(app: tauri::AppHandle, dir: String, screen: String, camera: Option<String>, mic: Option<String>) {
    if !begin(&dir) {
        return;
    }
    std::thread::spawn(move || {
        let r = build_tracked(&app, &dir, &screen, camera.as_deref());
        end(&dir);
        if let Err(e) = r {
            eprintln!("[proxy] background build failed: {e}");
        }
        if let Some(m) = mic {
            let _ = crate::levelmic::level(Path::new(&dir), &m);
        }
    });
}

/// Build in the background and report progress to the UI. If the build is already
/// running (started when the recording stopped), just follow it.
#[tauri::command]
pub async fn ensure_proxy(app: tauri::AppHandle, dir: String, screen: String, camera: Option<String>) -> Result<ProxyMeta, String> {
    use tauri::Emitter;
    tauri::async_runtime::spawn_blocking(move || {
        loop {
            if begin(&dir) {
                let r = build_tracked(&app, &dir, &screen, camera.as_deref());
                end(&dir);
                return r;
            }
            if let Some(p) = progress_of(&dir) {
                let _ = app.emit("proxy-progress", p);
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
