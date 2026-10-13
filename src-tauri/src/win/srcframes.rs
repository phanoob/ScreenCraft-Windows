//! Full-resolution source frames for export (`sc://localhost/src/<kind>?t=<ms>&d=<dir>`),
//! decoded with Media Foundation.
//!
//! One worker thread owns the readers. Export asks for frames in increasing time
//! order, so each request normally just pulls the next decoded frame; a backwards or
//! far-forward request seeks the reader to the requested time.
//!
//! Same public surface as the macOS `srcframes.rs`.

use crate::win::mf::{Frame, VideoReader};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

pub struct Request {
    pub dir: String,
    pub kind: String,
    pub t_ms: f64,
    pub quality: u8,
    pub reply: Box<dyn FnOnce(Option<Vec<u8>>) + Send>,
}

static TX: OnceLock<Sender<Request>> = OnceLock::new();

pub fn submit(req: Request) {
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<Request>();
        std::thread::Builder::new()
            .name("screencraft-srcframes".into())
            .spawn(move || worker(rx))
            .expect("spawn srcframes worker");
        tx
    });
    if let Err(e) = tx.send(req) {
        (e.0.reply)(None);
    }
}

type Shared = std::sync::Arc<Frame>;

struct Session {
    reader: VideoReader,
    /// The most recent decoded frame with pts <= the last request, and the one after it.
    cur: Option<Shared>,
    next: Option<Shared>,
    /// Frames just passed, so requests that arrive slightly out of order are still served.
    recent: VecDeque<Shared>,
    eof: bool,
    /// Time of the last forced reopen after the reader ran dry early.
    last_retry: f64,
}

impl Session {
    fn open(path: &str) -> Option<Session> {
        let reader = VideoReader::open(path, None).ok()?;
        Some(Session { reader, cur: None, next: None, recent: VecDeque::new(), eof: false, last_retry: -10.0 })
    }

    fn reopen(&mut self, start_s: f64) -> bool {
        if self.reader.seek(start_s).is_err() {
            return false;
        }
        self.cur = None;
        self.next = None;
        self.recent.clear();
        self.eof = false;
        true
    }

    fn pull(&mut self) -> Option<Shared> {
        self.reader.next().map(std::sync::Arc::new)
    }

    /// The decoded frame shown at `t` seconds (the latest one with pts <= t).
    fn frame_at(&mut self, t: f64) -> Option<Shared> {
        // slightly out of order: serve it from the frames we just passed
        if self.cur.as_ref().map(|c| t + 1e-4 < c.pts).unwrap_or(false) {
            if let Some(i) = self.recent.iter().rposition(|f| f.pts <= t + 1e-4) {
                return Some(self.recent[i].clone());
            }
        }
        // a request before what we already passed, or far ahead: restart there
        let behind = self.cur.as_ref().map(|c| t + 1e-4 < c.pts).unwrap_or(false);
        let far = self.cur.as_ref().map(|c| t - c.pts > 4.0).unwrap_or(false);
        if behind || far {
            self.reopen((t - 0.25).max(0.0));
        }
        // The reader can end early (a damaged or oddly timestamped file): a frame that
        // stopped well before `t` would freeze the picture, so reopen at `t` and carry on.
        // If nothing more can be read, keep showing the last good frame.
        let stalled = self.eof && self.cur.as_ref().map(|c| t - c.pts > 0.5).unwrap_or(false) && t - self.last_retry > 0.5;
        if stalled {
            self.last_retry = t;
            let held = self.cur.take();
            if self.reopen((t - 0.1).max(0.0)) {
                self.next = self.pull();
                if self.next.is_none() {
                    self.eof = true;
                }
            }
            if self.next.is_none() && self.cur.is_none() {
                self.cur = held;
                self.eof = true;
            }
        }
        if self.cur.is_none() && self.next.is_none() && !self.eof {
            self.next = self.pull();
            if self.next.is_none() {
                self.eof = true;
            }
        }
        loop {
            match &self.next {
                Some(n) if n.pts <= t + 1e-4 => {
                    if let Some(prev) = self.cur.take() {
                        self.recent.push_back(prev);
                        if self.recent.len() > 8 {
                            self.recent.pop_front();
                        }
                    }
                    self.cur = self.next.take();
                    self.next = self.pull();
                    if self.next.is_none() {
                        self.eof = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        if self.cur.is_none() {
            // t is before the first frame: show the first one
            self.cur = self.next.take();
            self.next = self.pull();
        }
        self.cur.clone()
    }
}

/// At most this many frames are JPEG-encoding at once (decode keeps running meanwhile).
const ENCODERS: usize = 6;
static INFLIGHT: (std::sync::Mutex<usize>, std::sync::Condvar) = (std::sync::Mutex::new(0), std::sync::Condvar::new());

fn worker(rx: std::sync::mpsc::Receiver<Request>) {
    crate::win::com_init();
    let mut sessions: HashMap<String, Session> = HashMap::new();
    while let Ok(req) = rx.recv() {
        let file = match req.kind.as_str() {
            "screen" => "screen",
            "camera" => "camera",
            _ => "",
        };
        let result = (|| {
            if file.is_empty() {
                return None;
            }
            let path = project_file(&req.dir, file)?;
            let key = format!("{}|{}", req.dir, req.kind);
            if !sessions.contains_key(&key) {
                sessions.insert(key.clone(), Session::open(&path)?);
            }
            sessions.get_mut(&key)?.frame_at(req.t_ms / 1000.0)
        })();
        match result {
            Some(frame) => {
                // encode on a helper thread so the next frame can be decoded right away
                {
                    let mut n = INFLIGHT.0.lock().unwrap();
                    while *n >= ENCODERS {
                        n = INFLIGHT.1.wait(n).unwrap();
                    }
                    *n += 1;
                }
                let q = req.quality;
                let reply = req.reply;
                std::thread::spawn(move || {
                    let out = crate::jpegio::encode_bgra(frame.data.clone(), frame.w as usize, frame.h as usize, q);
                    {
                        let mut n = INFLIGHT.0.lock().unwrap();
                        *n -= 1;
                        INFLIGHT.1.notify_one();
                    }
                    reply(out);
                });
            }
            None => (req.reply)(None),
        }
    }
}

/// Resolve "screen"/"camera" to the file named in project.json.
fn project_file(dir: &str, which: &str) -> Option<String> {
    let bytes = std::fs::read(std::path::Path::new(dir).join("project.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let name = v["media"][which].as_str()?;
    Some(std::path::Path::new(dir).join(name).to_string_lossy().into_owned())
}
