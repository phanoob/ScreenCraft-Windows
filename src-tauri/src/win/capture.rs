//! Screen capture on Windows: Windows Graphics Capture delivers BGRA frames, a
//! writer thread encodes them with Media Foundation (H.264 + AAC system audio from
//! WASAPI loopback) into screen.mp4, while the input tracker logs the cursor.
//!
//! Same public surface as the macOS `capture.rs`.

use crate::inputlog::InputTracker;
use crate::win::{displays, loopback, mf};
use screencraft_core::project::Rect;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOpts {
    /// "display" | "window" | "area"
    pub mode: String,
    pub display_id: Option<u32>,
    pub window_id: Option<u32>,
    pub area: Option<AreaRect>,
    #[serde(default)]
    pub system_audio: bool,
    #[serde(default)]
    pub microphone: bool,
    #[serde(default)]
    pub microphone_id: Option<String>,
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// Cap for the long side of the recording in pixels; `None` = native resolution.
    #[serde(default)]
    pub max_long_side: Option<u32>,
    pub output_dir: PathBuf,
}

fn default_fps() -> u32 {
    60
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartInfo {
    pub width: u32,
    pub height: u32,
    pub bounds: Rect,
    /// Wall clock (ms since epoch) at which the first frame was recorded.
    pub started_unix_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopInfo {
    pub dir: PathBuf,
    pub duration_ms: f64,
    pub width: u32,
    pub height: u32,
    pub bounds: Rect,
    pub started_unix_ms: f64,
    pub has_system_audio: bool,
    pub has_microphone: bool,
}

enum Cmd {
    Start(StartOpts, Sender<Result<StartInfo, String>>),
    Stop(Sender<Result<StopInfo, String>>),
}

#[derive(Clone)]
pub struct Capture {
    tx: Sender<Cmd>,
}

impl Capture {
    pub fn spawn() -> Self {
        let (tx, rx) = channel();
        thread::Builder::new()
            .name("screencraft-capture".into())
            .spawn(move || run(rx))
            .expect("capture thread");
        Self { tx }
    }

    pub fn start(&self, opts: StartOpts) -> Result<StartInfo, String> {
        let (rtx, rrx) = channel();
        self.tx.send(Cmd::Start(opts, rtx)).map_err(|e| e.to_string())?;
        rrx.recv_timeout(Duration::from_secs(20)).map_err(|e| format!("capture start timed out: {e}"))?
    }

    pub fn stop(&self) -> Result<StopInfo, String> {
        let (rtx, rrx) = channel();
        self.tx.send(Cmd::Stop(rtx)).map_err(|e| e.to_string())?;
        rrx.recv_timeout(Duration::from_secs(60)).map_err(|e| format!("capture stop timed out: {e}"))?
    }
}

// ---------------------------------------------------------------- writer thread

enum Msg {
    /// Packed BGRA at the output size, and when it arrived.
    Video(Vec<u8>, Instant),
    Audio(Vec<f32>, Instant),
    Stop(Instant),
}

/// First-frame instant, shared by the capture callback, the audio thread and the writer.
type Zero = Arc<(Mutex<Option<Instant>>, Condvar)>;

struct WriterResult {
    duration_s: f64,
}

/// Owns the sink writer (it is only touched from this thread).
fn writer_loop(
    rx: Receiver<Msg>,
    path: PathBuf,
    spec: mf::VideoSpec,
    audio: bool,
    zero: Zero,
) -> Result<WriterResult, String> {
    crate::win::com_init();
    let aspec = mf::AudioSpec { rate: loopback::RATE, channels: loopback::CHANNELS, kbps: 192 };
    let fps = spec.fps;
    let mut w = mf::Writer::create(&path.to_string_lossy(), &spec, audio.then_some(&aspec), false)?;
    let zero_at = || *zero.0.lock().unwrap();

    let rate = loopback::RATE as f64;
    let ch = loopback::CHANNELS as usize;
    let mut audio_frames: u64 = 0; // stereo frames written so far
    let mut last_frame: Option<Vec<u8>> = None;
    let mut last_t = 0.0f64; // pts of the newest video frame written
    let mut error: Option<String> = None;

    let write_silence = |w: &mut mf::Writer, frames: &mut u64, until: f64| {
        // in 100 ms blocks so a long gap does not allocate a huge buffer
        while (*frames as f64) / rate + 0.0005 < until {
            let n = ((until * rate) as u64 - *frames).min(4800).max(1);
            let zeros = vec![0i16; n as usize * ch];
            let _ = w.write_audio(&zeros, *frames as f64 / rate);
            *frames += n;
        }
    };

    let end_t = loop {
        let msg = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break last_t,
        };
        match msg {
            Some(Msg::Video(px, at)) => {
                let Some(z) = zero_at() else { continue };
                let t = at.saturating_duration_since(z).as_secs_f64();
                if t + 1e-4 < last_t {
                    continue;
                }
                if let Err(e) = w.write_video(&px, t) {
                    error.get_or_insert(e);
                }
                last_t = t;
                last_frame = Some(px);
            }
            Some(Msg::Audio(samples, at)) => {
                let Some(z) = zero_at() else { continue };
                let len_s = samples.len() as f64 / ch as f64 / rate;
                let start = at.saturating_duration_since(z).as_secs_f64() - len_s;
                let written = audio_frames as f64 / rate;
                // the loopback stream goes quiet while nothing plays: fill the gap
                if start > written + 0.05 {
                    write_silence(&mut w, &mut audio_frames, start);
                }
                // drop what overlaps audio already written (silence padded ahead, or
                // anything from before the first video frame), so sound stays in sync
                let overlap = written - start;
                let skip = if overlap > 0.02 { (overlap * rate) as usize * ch } else { 0 };
                if skip < samples.len() {
                    let pcm: Vec<i16> = samples[skip..].iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
                    let _ = w.write_audio(&pcm, audio_frames as f64 / rate);
                    audio_frames += (pcm.len() / ch) as u64;
                }
            }
            Some(Msg::Stop(at)) => {
                break zero_at().map(|z| at.saturating_duration_since(z).as_secs_f64()).unwrap_or(last_t);
            }
            None => {}
        }
        let Some(z) = zero_at() else { continue };
        let now = Instant::now().saturating_duration_since(z).as_secs_f64();
        // Windows only delivers frames when something changes on screen: repeat the
        // last one now and then so a still screen still has a steady timeline.
        if let Some(px) = &last_frame {
            if now - last_t > 1.0 {
                let _ = w.write_video(px, now);
                last_t = now;
            }
        }
        // keep the audio track moving through silences (loopback packets arrive
        // within a few ms, so anything 300 ms old is not coming)
        if audio && now - 0.3 > audio_frames as f64 / rate + 0.05 {
            write_silence(&mut w, &mut audio_frames, now - 0.3);
        }
    };

    // close the timeline at the moment recording stopped
    if let Some(px) = &last_frame {
        if end_t > last_t + 1.0 / fps {
            let _ = w.write_video(px, end_t);
        }
    }
    if audio {
        write_silence(&mut w, &mut audio_frames, end_t);
    }
    w.finish()?;
    if let Some(e) = error {
        eprintln!("[capture] encoder reported: {e}");
    }
    if last_frame.is_none() {
        return Err("no frames were captured".into());
    }
    Ok(WriterResult { duration_s: end_t.max(last_t) })
}

// ---------------------------------------------------------------- capture handler

struct Flags {
    tx: SyncSender<Msg>,
    zero: Zero,
    /// Crop rectangle inside the captured surface (physical pixels); `None` = all.
    crop: Option<(u32, u32, u32, u32)>,
    out_w: u32,
    out_h: u32,
    min_interval: Duration,
    /// Frames dropped because the encoder was behind (reported when recording stops).
    dropped: Arc<AtomicU64>,
}

struct Handler {
    f: Flags,
    last: Option<Instant>,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { f: ctx.flags, last: None, scratch: Vec::new() })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
        let now = Instant::now();
        if let Some(l) = self.last {
            if now.duration_since(l) < self.f.min_interval {
                return Ok(());
            }
        }
        let (fw, fh) = (frame.width(), frame.height());
        let buf = frame.buffer()?;
        let raw = buf.as_nopadding_buffer(&mut self.scratch);
        // crop (area mode), clamped to the surface
        let (cx, cy, cw, ch) = self.f.crop.unwrap_or((0, 0, fw, fh));
        let cx = cx.min(fw.saturating_sub(2));
        let cy = cy.min(fh.saturating_sub(2));
        let cw = cw.min(fw - cx).max(2);
        let ch = ch.min(fh - cy).max(2);
        let mut px = vec![0u8; (cw * ch * 4) as usize];
        let row = (cw * 4) as usize;
        for y in 0..ch as usize {
            let s = ((cy as usize + y) * fw as usize + cx as usize) * 4;
            px[y * row..(y + 1) * row].copy_from_slice(&raw[s..s + row]);
        }
        let px = mf::resize_bgra(&px, cw as usize, ch as usize, self.f.out_w as usize, self.f.out_h as usize);

        // the first frame defines t = 0 for video, audio and the input log
        {
            let (m, cv) = &*self.f.zero;
            let mut z = m.lock().unwrap();
            if z.is_none() {
                *z = Some(now);
                cv.notify_all();
            }
        }
        match self.f.tx.try_send(Msg::Video(px, now)) {
            Ok(()) => self.last = Some(now),
            Err(TrySendError::Full(_)) => {
                // encoder is behind: drop this frame
                self.f.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- session

struct Session {
    control: CaptureControl<Handler, Box<dyn std::error::Error + Send + Sync>>,
    audio: Option<loopback::Loopback>,
    writer: JoinHandle<Result<WriterResult, String>>,
    tx: SyncSender<Msg>,
    tracker: InputTracker,
    zero: Zero,
    dir: PathBuf,
    bounds: Rect,
    width: u32,
    height: u32,
    started_unix_ms: f64,
    system_audio: bool,
    dropped: Arc<AtomicU64>,
}

fn unix_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn even(v: f64) -> u32 {
    let n = v.round().max(2.0) as u32;
    n - (n % 2)
}

fn run(rx: Receiver<Cmd>) {
    let mut session: Option<Session> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Start(opts, reply) => {
                if session.is_some() {
                    let _ = reply.send(Err("already recording".into()));
                    continue;
                }
                match begin(opts) {
                    Ok(s) => {
                        let info = StartInfo {
                            width: s.width,
                            height: s.height,
                            bounds: s.bounds,
                            started_unix_ms: s.started_unix_ms,
                        };
                        session = Some(s);
                        let _ = reply.send(Ok(info));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Cmd::Stop(reply) => {
                let Some(s) = session.take() else {
                    let _ = reply.send(Err("not recording".into()));
                    continue;
                };
                let _ = reply.send(end(s));
            }
        }
    }
}

/// H.264 encoders top out around 4096 pixels on the long side.
const MAX_ENCODE_SIDE: f64 = 4096.0;

fn begin(opts: StartOpts) -> Result<Session, String> {
    mf::startup();
    let display = || displays::find(opts.display_id).ok_or_else(|| "display not found".to_string());

    // What to capture, the captured rect in physical pixels and the crop inside it.
    enum Item {
        Monitor(isize),
        Window(isize),
    }
    let (item, bounds, crop) = match opts.mode.as_str() {
        "window" => {
            let id = opts.window_id.ok_or("window id missing")?;
            let hwnd = id as isize;
            let (x, y, w, h) = displays::window_rect(hwnd).ok_or("window not found")?;
            (Item::Window(hwnd), Rect { x, y, width: w, height: h }, None)
        }
        "area" => {
            let a = opts.area.clone().ok_or("area missing")?;
            // the area is in display points of the display it was drawn on
            let d = displays::list()
                .into_iter()
                .find(|d| d.contains_point(a.x, a.y))
                .map(Ok)
                .unwrap_or_else(display)?;
            let s = d.scale;
            let (x, y) = ((a.x * s).round(), (a.y * s).round());
            let (w, h) = ((a.width * s).round(), (a.height * s).round());
            let crop = ((x - d.px.0).max(0.0) as u32, (y - d.px.1).max(0.0) as u32, w as u32, h as u32);
            (Item::Monitor(d.hmonitor), Rect { x, y, width: w, height: h }, Some(crop))
        }
        _ => {
            let d = display()?;
            (Item::Monitor(d.hmonitor), Rect { x: d.px.0, y: d.px.1, width: d.px.2, height: d.px.3 }, None)
        }
    };

    // Resolution is chosen by the user as a cap on the long side (never upscaled).
    let long = bounds.width.max(bounds.height).max(1.0);
    let cap = opts.max_long_side.filter(|m| *m > 0).map(|m| m as f64).unwrap_or(f64::MAX).min(MAX_ENCODE_SIDE);
    let scale = (cap / long).min(1.0);
    let (width, height) = (even(bounds.width * scale), even(bounds.height * scale));
    let fps = opts.fps.clamp(10, 120);

    std::fs::create_dir_all(&opts.output_dir).map_err(|e| e.to_string())?;
    let movie = opts.output_dir.join("screen.mp4");

    let zero: Zero = Arc::new((Mutex::new(None), Condvar::new()));
    let (tx, rx) = sync_channel::<Msg>(8);
    let spec = mf::VideoSpec {
        width,
        height,
        fps: fps as f64,
        // generous: screen content is mostly static, motion needs headroom
        bitrate: ((width as f64 * height as f64 * fps as f64 * 0.12).clamp(6e6, 60e6)) as u32,
        hevc: false,
        gop: fps * 2,
    };
    let (z, path, sys) = (zero.clone(), movie.clone(), opts.system_audio);
    let writer = thread::Builder::new()
        .name("screencraft-encoder".into())
        .spawn(move || writer_loop(rx, path, spec, sys, z))
        .map_err(|e| e.to_string())?;

    let dropped = Arc::new(AtomicU64::new(0));
    let flags = Flags {
        dropped: dropped.clone(),
        tx: tx.clone(),
        zero: zero.clone(),
        crop,
        out_w: width,
        out_h: height,
        min_interval: Duration::from_secs_f64(0.92 / fps as f64),
    };
    let interval = MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / fps as f64));
    let control = match item {
        Item::Monitor(h) => Handler::start_free_threaded(Settings::new(
            Monitor::from_raw_hmonitor(h as *mut std::ffi::c_void),
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            interval,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            flags,
        )),
        Item::Window(h) => Handler::start_free_threaded(Settings::new(
            Window::from_raw_hwnd(h as *mut std::ffi::c_void),
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Include,
            interval,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            flags,
        )),
    }
    .map_err(|e| format!("screen capture failed to start: {e}"))?;

    let tracker = InputTracker::start();

    // Wait for the first frame so cursor times line up with the video. A still
    // screen may not produce one right away, so the wait is generous.
    let started_unix_ms = {
        let (m, cv) = &*zero;
        let guard = m.lock().unwrap();
        let (guard, _) = cv.wait_timeout_while(guard, Duration::from_secs(5), |z| z.is_none()).unwrap();
        match *guard {
            Some(z) => unix_ms() - z.elapsed().as_secs_f64() * 1000.0,
            None => {
                drop(guard);
                let _ = control.stop();
                let _ = tracker.finish(Instant::now(), bounds);
                return Err("screen capture did not start".into());
            }
        }
    };

    let audio = if opts.system_audio {
        let atx = tx.clone();
        match loopback::Loopback::start(move |s, at| {
            let _ = atx.send(Msg::Audio(s.to_vec(), at));
        }) {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("[capture] {e}");
                None
            }
        }
    } else {
        None
    };

    Ok(Session {
        control,
        audio,
        writer,
        tx,
        tracker,
        zero,
        dir: opts.output_dir,
        bounds,
        width,
        height,
        started_unix_ms,
        system_audio: opts.system_audio,
        dropped,
    })
}

fn end(s: Session) -> Result<StopInfo, String> {
    let stop_at = Instant::now();
    let zero = s.zero.0.lock().unwrap().unwrap_or(stop_at);
    let _ = s.control.stop();
    if let Some(a) = s.audio {
        a.stop();
    }
    let _ = s.tx.send(Msg::Stop(stop_at));
    drop(s.tx);
    let result = s.writer.join().map_err(|_| "encoder thread panicked".to_string())??;
    let dropped = s.dropped.load(Ordering::Relaxed);
    if dropped > 0 {
        eprintln!("[capture] the encoder fell behind: {dropped} frames dropped");
    }
    let duration_ms = result.duration_s * 1000.0;

    let log = s.tracker.finish(zero, s.bounds);
    let json = serde_json::to_vec(&log).map_err(|e| e.to_string())?;
    std::fs::write(s.dir.join("input.json"), json).map_err(|e| e.to_string())?;
    Ok(StopInfo {
        dir: s.dir,
        duration_ms,
        width: s.width,
        height: s.height,
        bounds: s.bounds,
        started_unix_ms: s.started_unix_ms,
        has_system_audio: s.system_audio,
        has_microphone: false,
    })
}
