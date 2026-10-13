//! Live system-audio level meter on Windows: a WASAPI loopback stream feeds a peak
//! value that is pushed to the UI as `system-level` events, so the toolbar can show
//! that audio is really being captured before you record.
//!
//! Same public surface as the macOS `meter.rs`.

use crate::win::loopback::Loopback;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

struct Running {
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct Meter {
    running: Mutex<Option<Running>>,
}

impl Meter {
    pub fn start(&self, app: AppHandle) -> Result<(), String> {
        let mut guard = self.running.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }
        let peak = Arc::new(AtomicU32::new(0));
        let p = peak.clone();
        let stream = Loopback::start(move |samples, _| {
            let v = samples.iter().fold(0.0f32, |m, s| if s.is_finite() { m.max(s.abs()) } else { m });
            // keep the largest value seen since the UI last read it
            let _ = p.try_update(Ordering::Relaxed, Ordering::Relaxed, |old| (f32::from_bits(old) < v).then_some(v.to_bits()));
        })?;

        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        thread::Builder::new()
            .name("screencraft-meter".into())
            .spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    let v = f32::from_bits(peak.swap(0, Ordering::Relaxed));
                    let _ = app.emit("system-level", v);
                    thread::sleep(Duration::from_millis(60));
                }
                stream.stop();
            })
            .map_err(|e| e.to_string())?;
        *guard = Some(Running { stop });
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(r) = self.running.lock().unwrap().take() {
            r.stop.store(true, Ordering::Relaxed);
        }
    }
}
