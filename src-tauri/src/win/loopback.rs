//! System audio through WASAPI loopback on the default output device, converted by
//! the audio engine to 48 kHz stereo float. Used by the recorder (muxed into
//! screen.mp4) and by the live level meter.
//!
//! Loopback delivers nothing at all while no app is playing sound, so callers get
//! the wall-clock instant of every packet and pad gaps with silence themselves.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

pub const RATE: u32 = 48_000;
pub const CHANNELS: u32 = 2;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

pub struct Loopback {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Loopback {
    /// `on_data(samples, at)`: interleaved stereo f32 at 48 kHz; `at` is when the
    /// packet arrived (its last sample was played roughly then).
    pub fn start(mut on_data: impl FnMut(&[f32], Instant) + Send + 'static) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread = thread::Builder::new()
            .name("screencraft-loopback".into())
            .spawn(move || {
                super::com_init();
                let (client, capture) = match open() {
                    Ok(v) => {
                        let _ = ready_tx.send(Ok(()));
                        v
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let mut tmp: Vec<f32> = Vec::new();
                while !s.load(Ordering::SeqCst) {
                    unsafe {
                        loop {
                            match capture.GetNextPacketSize() {
                                Ok(n) if n > 0 => {}
                                _ => break,
                            }
                            let mut data: *mut u8 = std::ptr::null_mut();
                            let (mut frames, mut flags) = (0u32, 0u32);
                            if capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                                break;
                            }
                            let n = (frames * CHANNELS) as usize;
                            tmp.clear();
                            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                                tmp.resize(n, 0.0);
                            } else {
                                tmp.extend_from_slice(std::slice::from_raw_parts(data as *const f32, n));
                            }
                            let _ = capture.ReleaseBuffer(frames);
                            on_data(&tmp, Instant::now());
                        }
                    }
                    thread::sleep(Duration::from_millis(8));
                }
                unsafe {
                    let _ = client.Stop();
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "system audio did not start".to_string())??;
        Ok(Self { stop, thread: Some(thread) })
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn open() -> Result<(IAudioClient, IAudioCaptureClient), String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| format!("no audio output device: {e}"))?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        let fmt = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
            nChannels: CHANNELS as u16,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * CHANNELS * 4,
            nBlockAlign: (CHANNELS * 4) as u16,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                2_000_000, // 200 ms buffer
                0,
                &fmt,
                None,
            )
            .map_err(|e| format!("system audio capture failed: {e}"))?;
        let capture: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
        client.Start().map_err(|e| e.to_string())?;
        Ok((client, capture))
    }
}
