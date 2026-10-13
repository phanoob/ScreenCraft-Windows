//! Global mouse / keyboard tracker for Windows. Polls the cursor position, button
//! state and cursor shape at ~120 Hz; keystrokes come from a low-level keyboard
//! hook. Positions are physical pixels (the app is per-monitor DPI aware).
//!
//! Same public surface as the macOS `inputlog.rs`.

use screencraft_core::input::{Button, CaretEvent, ClickEvent, CursorKind, InputLog, KeyEvent, MoveEvent};
use screencraft_core::project::Rect;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorInfo, GetMessageW, LoadCursorW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, CURSORINFO, HCURSOR, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_NO,
    IDC_SIZEALL, IDC_SIZENS, IDC_SIZEWE, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// macOS virtual key codes for the keys the core treats specially
/// (see `InputLog::refine_carets`); everything else is logged as "vk<code>".
fn mac_key(vk: u32) -> String {
    match vk {
        0x08 => "51".into(),  // Backspace
        0x0D => "36".into(),  // Return
        0x09 => "48".into(),  // Tab
        0x1B => "53".into(),  // Escape
        0x25 => "123".into(), // Left
        0x27 => "124".into(), // Right
        0x28 => "125".into(), // Down
        0x26 => "126".into(), // Up
        0x2E => "117".into(), // Delete
        _ => format!("vk{vk}"),
    }
}

/// Modifier-only keys never count as typing.
fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5)
}

fn button_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

struct Cursors {
    table: Vec<(isize, CursorKind)>,
}

impl Cursors {
    fn load() -> Self {
        let pairs = [
            (IDC_IBEAM, CursorKind::IBeam),
            (IDC_HAND, CursorKind::Pointer),
            (IDC_CROSS, CursorKind::Crosshair),
            (IDC_SIZEALL, CursorKind::OpenHand),
            (IDC_SIZEWE, CursorKind::ResizeLeftRight),
            (IDC_SIZENS, CursorKind::ResizeUpDown),
            (IDC_NO, CursorKind::NotAllowed),
            (IDC_ARROW, CursorKind::Arrow),
        ];
        let table = pairs
            .iter()
            .filter_map(|(id, kind)| unsafe { LoadCursorW(None, *id).ok().map(|h: HCURSOR| (h.0 as isize, *kind)) })
            .collect();
        Self { table }
    }

    /// Current cursor shape and position (physical pixels).
    fn current(&self) -> Option<(f64, f64, CursorKind)> {
        let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        unsafe { GetCursorInfo(&mut ci).ok()? };
        let h = ci.hCursor.0 as isize;
        let kind = self.table.iter().find(|(c, _)| *c == h).map(|(_, k)| *k).unwrap_or(CursorKind::Arrow);
        let POINT { x, y } = ci.ptScreenPos;
        Some((x as f64, y as f64, kind))
    }
}

#[derive(Clone, Copy)]
struct Sample {
    at: Instant,
    x: f64,
    y: f64,
    cursor: CursorKind,
}

#[derive(Clone, Copy)]
struct Click {
    at: Instant,
    x: f64,
    y: f64,
    button: Button,
    down: bool,
}

#[derive(Clone)]
struct Key {
    at: Instant,
    code: String,
}

#[derive(Clone, Copy)]
struct Caret {
    at: Instant,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Default)]
struct Buffers {
    samples: Vec<Sample>,
    clicks: Vec<Click>,
    keys: Vec<Key>,
    carets: Vec<Caret>,
}

pub struct InputTracker {
    stop: Arc<AtomicBool>,
    buffers: Arc<Mutex<Buffers>>,
    thread: Option<JoinHandle<()>>,
    key_thread: Option<JoinHandle<()>>,
    key_thread_id: Arc<AtomicU32>,
}

// The low-level hook procedure has no user data pointer, so it reaches the active
// tracker's buffers through this slot.
static KEY_SINK: Mutex<Option<Arc<Mutex<Buffers>>>> = Mutex::new(None);

/// Keys currently held down (bit per virtual key), so auto-repeat is not counted.
static HELD: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];

unsafe extern "system" fn key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = k.vkCode & 0xFF;
        let (word, bit) = ((vk / 32) as usize, 1u32 << (vk % 32));
        let msg = wparam.0 as u32;
        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            let was_held = HELD[word].fetch_or(bit, Ordering::Relaxed) & bit != 0;
            if !was_held && !is_modifier(vk) {
                if let Ok(g) = KEY_SINK.try_lock() {
                    if let Some(b) = g.as_ref() {
                        b.lock().unwrap().keys.push(Key { at: Instant::now(), code: mac_key(vk) });
                    }
                }
            }
        } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
            HELD[word].fetch_and(!bit, Ordering::Relaxed);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Keyboard activity through a low-level hook; needs a message loop on its thread.
fn key_hook_loop(thread_id: Arc<AtomicU32>) {
    unsafe {
        thread_id.store(GetCurrentThreadId(), Ordering::SeqCst);
        let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), None, 0) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[screencraft] keyboard hook unavailable: {e}");
                return;
            }
        };
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
}

impl InputTracker {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let buffers = Arc::new(Mutex::new(Buffers::default()));
        *KEY_SINK.lock().unwrap() = Some(buffers.clone());
        let (s, b) = (stop.clone(), buffers.clone());
        let thread = thread::Builder::new()
            .name("screencraft-input".into())
            .spawn(move || poll_loop(s, b))
            .ok();
        let key_thread_id = Arc::new(AtomicU32::new(0));
        let tid = key_thread_id.clone();
        let key_thread = thread::Builder::new()
            .name("screencraft-keys".into())
            .spawn(move || key_hook_loop(tid))
            .ok();
        Self { stop, buffers, thread, key_thread, key_thread_id }
    }

    /// Stop polling and convert the log to times relative to `zero`.
    pub fn finish(mut self, zero: Instant, bounds: Rect) -> InputLog {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.key_thread.take() {
            // the hook thread may not have published its id yet
            for _ in 0..100 {
                let id = self.key_thread_id.load(Ordering::SeqCst);
                if id != 0 {
                    unsafe {
                        let _ = PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0));
                    }
                    break;
                }
                if t.is_finished() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            let _ = t.join();
        }
        *KEY_SINK.lock().unwrap() = None;
        let buf = std::mem::take(&mut *self.buffers.lock().unwrap());
        let rel = |at: Instant| at.saturating_duration_since(zero).as_secs_f64() * 1000.0;
        InputLog {
            bounds,
            moves: buf
                .samples
                .iter()
                .map(|s| MoveEvent { t: rel(s.at), x: s.x, y: s.y, cursor: s.cursor })
                .collect(),
            clicks: buf
                .clicks
                .iter()
                .map(|c| ClickEvent { t: rel(c.at), x: c.x, y: c.y, button: c.button, down: c.down })
                .collect(),
            keys: buf
                .keys
                .iter()
                .map(|k| KeyEvent { t: rel(k.at), key: k.code.clone(), down: true })
                .collect(),
            carets: buf
                .carets
                .iter()
                .map(|c| CaretEvent { t: rel(c.at), x: c.x, y: c.y, w: c.w, h: c.h })
                .collect(),
        }
    }
}

fn poll_loop(stop: Arc<AtomicBool>, buffers: Arc<Mutex<Buffers>>) {
    crate::win::com_init();
    let cursors = Cursors::load();
    let mut last_pos = (f64::NAN, f64::NAN);
    let mut left = false;
    let mut right = false;
    let mut tick = 0u32;
    let mut finder = crate::ax::CaretFinder::default();
    while !stop.load(Ordering::SeqCst) {
        let now = Instant::now();
        if let Some((x, y, last_kind)) = cursors.current() {
            let (l, r) = (button_down(VK_LBUTTON.0 as i32), button_down(VK_RBUTTON.0 as i32));
            let mut b = buffers.lock().unwrap();
            let moved = x != last_pos.0 || y != last_pos.1;
            let kind_changed = b.samples.last().map(|s| s.cursor != last_kind).unwrap_or(true);
            if moved || kind_changed || b.samples.is_empty() {
                b.samples.push(Sample { at: now, x, y, cursor: last_kind });
                last_pos = (x, y);
            }
            if l != left {
                left = l;
                b.clicks.push(Click { at: now, x, y, button: Button::Left, down: l });
            }
            if r != right {
                right = r;
                b.clicks.push(Click { at: now, x, y, button: Button::Right, down: r });
            }
        }
        // While typing, follow the text caret (~16 Hz).
        if tick % 7 == 0 {
            let typing = buffers
                .lock()
                .unwrap()
                .keys
                .last()
                .map(|k| now.duration_since(k.at) < Duration::from_millis(2000))
                .unwrap_or(false);
            if typing {
                if let Some((x, y, w, h)) = finder.caret() {
                    buffers.lock().unwrap().carets.push(Caret { at: now, x, y, w, h });
                }
            }
        }
        tick = tick.wrapping_add(1);
        thread::sleep(Duration::from_micros(8000));
    }
}
