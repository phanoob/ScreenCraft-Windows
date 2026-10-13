//! Text-caret lookup on Windows: the Win32 system caret first (classic edit
//! controls), then UI Automation's TextPattern2 caret range (modern apps, browsers,
//! Electron), then the focused element's frame when it is small. Rectangles are in
//! physical screen pixels. No permission is needed on Windows.
//!
//! Same public surface as the macOS `ax.rs`.

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, SAFEARRAY};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern2, IUIAutomationTextRange, TextPatternRangeEndpoint_End,
    TextPatternRangeEndpoint_Start, TextUnit_Character, UIA_TextPattern2Id,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO};

/// Finds the caret of whatever app is in front.
#[derive(Default)]
pub struct CaretFinder {
    uia: Option<IUIAutomation>,
    tried_uia: bool,
}

type R = (f64, f64, f64, f64);

fn usable(r: R) -> bool {
    r.3 > 0.0 && r.0.is_finite() && r.1.is_finite() && (r.0 != 0.0 || r.1 != 0.0)
}

impl CaretFinder {
    /// Caret rectangle in global physical pixels (top-left origin).
    pub fn caret(&mut self) -> Option<R> {
        if let Some(r) = system_caret() {
            return Some(r);
        }
        if !self.tried_uia {
            self.tried_uia = true;
            self.uia = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
        }
        let uia = self.uia.as_ref()?;
        unsafe { uia_caret(uia) }
    }
}

/// The classic Win32 caret of the foreground thread (Notepad-style edit controls).
fn system_caret() -> Option<R> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return None;
        }
        let tid = GetWindowThreadProcessId(fg, None);
        let mut gti = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(tid, &mut gti).ok()?;
        if gti.hwndCaret.is_invalid() {
            return None;
        }
        let rc = gti.rcCaret;
        let mut p = POINT { x: rc.left, y: rc.top };
        if !ClientToScreen(gti.hwndCaret, &mut p).as_bool() {
            return None;
        }
        let r = (p.x as f64, p.y as f64, ((rc.right - rc.left) as f64).clamp(1.0, 3.0), (rc.bottom - rc.top) as f64);
        usable(r).then_some(r)
    }
}

/// First rectangle of a text range (left, top, width, height).
unsafe fn first_rect(range: &IUIAutomationTextRange) -> Option<R> {
    let arr: *mut SAFEARRAY = range.GetBoundingRectangles().ok()?;
    if arr.is_null() {
        return None;
    }
    let mut out = None;
    if let Ok(ub) = SafeArrayGetUBound(arr, 1) {
        if ub >= 3 {
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            if SafeArrayAccessData(arr, &mut data).is_ok() {
                let d = std::slice::from_raw_parts(data as *const f64, 4);
                out = Some((d[0], d[1], d[2], d[3]));
                let _ = SafeArrayUnaccessData(arr);
            }
        }
    }
    let _ = SafeArrayDestroy(arr);
    out
}

unsafe fn uia_caret(uia: &IUIAutomation) -> Option<R> {
    let el = uia.GetFocusedElement().ok()?;
    if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
        let mut active = windows::core::BOOL(0);
        if let Ok(caret) = tp.GetCaretRange(&mut active) {
            // a collapsed range has no rectangle: measure the character after the
            // caret, else the one before it (and use its right edge)
            if let Some(r) = first_rect(&caret).filter(|r| usable(*r)) {
                return Some((r.0, r.1, r.2.min(3.0), r.3));
            }
            if let Ok(next) = caret.Clone() {
                if next.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 1).is_ok() {
                    if let Some(r) = first_rect(&next).filter(|r| usable(*r)) {
                        return Some((r.0, r.1, 2.0, r.3));
                    }
                }
            }
            if let Ok(prev) = caret.Clone() {
                if prev.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).is_ok() {
                    if let Some(r) = first_rect(&prev).filter(|r| usable(*r)) {
                        return Some((r.0 + r.2, r.1, 2.0, r.3));
                    }
                }
            }
        }
    }
    // fall back to the focused element's frame when it is reasonably small
    let b = el.CurrentBoundingRectangle().ok()?;
    let (w, h) = ((b.right - b.left) as f64, (b.bottom - b.top) as f64);
    (w > 0.0 && w <= 1100.0 && h > 0.0 && h <= 420.0).then_some((b.left as f64, b.top as f64, w, h))
}
