//! Monitors and top-level windows.
//!
//! Coordinates: Windows works in physical pixels (the app is per-monitor DPI aware).
//! The UI gets "display points": physical pixels divided by the scale factor of the
//! monitor they are on, which is what a web page on that monitor sees as CSS pixels.
//! Recordings (bounds, cursor log, carets) stay in physical pixels throughout.

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, GA_ROOTOWNER, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
};

#[derive(Clone, Debug)]
pub struct Display {
    pub id: u32,
    pub primary: bool,
    /// Physical pixels: x, y, width, height.
    pub px: (f64, f64, f64, f64),
    pub scale: f64,
    pub hmonitor: isize,
}

impl Display {
    /// The same rectangle in display points (see module docs).
    pub fn points(&self) -> (f64, f64, f64, f64) {
        let s = self.scale;
        (self.px.0 / s, self.px.1 / s, self.px.2 / s, self.px.3 / s)
    }

    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        let (dx, dy, dw, dh) = self.points();
        x >= dx && x < dx + dw && y >= dy && y < dy + dh
    }
}

fn monitor_scale(m: HMONITOR) -> f64 {
    let (mut dx, mut dy) = (96u32, 96u32);
    unsafe {
        if GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_err() {
            return 1.0;
        }
    }
    (dx as f64 / 96.0).max(0.5)
}

unsafe extern "system" fn monitor_cb(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<Display>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(m, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        let r = info.monitorInfo.rcMonitor;
        let name = String::from_utf16_lossy(&info.szDevice);
        let name = name.trim_end_matches('\0');
        // "\\.\DISPLAY2" -> 2: stable across launches, unlike the HMONITOR
        let digits: String = name.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        let id = digits.parse::<u32>().unwrap_or(list.len() as u32 + 1);
        list.push(Display {
            id,
            primary: info.monitorInfo.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
            px: (r.left as f64, r.top as f64, (r.right - r.left) as f64, (r.bottom - r.top) as f64),
            scale: monitor_scale(m),
            hmonitor: m.0 as isize,
        });
    }
    BOOL(1)
}

/// All monitors, the primary one first.
pub fn list() -> Vec<Display> {
    let mut out: Vec<Display> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(monitor_cb), LPARAM(&mut out as *mut Vec<Display> as isize));
    }
    out.sort_by_key(|d| (!d.primary, d.id));
    out
}

pub fn find(id: Option<u32>) -> Option<Display> {
    let all = list();
    match id {
        Some(id) => all.iter().find(|d| d.id == id).cloned().or_else(|| all.first().cloned()),
        None => all.first().cloned(),
    }
}

/// Scale factor of the monitor nearest to a physical point.
pub fn scale_at(x: f64, y: f64) -> f64 {
    unsafe {
        let m = MonitorFromPoint(windows::Win32::Foundation::POINT { x: x as i32, y: y as i32 }, MONITOR_DEFAULTTONEAREST);
        monitor_scale(m)
    }
}

#[derive(Clone, Debug)]
pub struct WinInfo {
    pub hwnd: isize,
    pub title: String,
    pub app: String,
    pub pid: u32,
    /// Physical pixels (visible frame, without the invisible resize borders).
    pub px: (f64, f64, f64, f64),
}

pub fn window_rect(hwnd: isize) -> Option<(f64, f64, f64, f64)> {
    let h = HWND(hwnd as _);
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32)
            .ok()?;
    }
    Some((r.left as f64, r.top as f64, (r.right - r.left) as f64, (r.bottom - r.top) as f64))
}

fn process_name(pid: u32) -> String {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

unsafe extern "system" fn window_cb(h: HWND, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<WinInfo>);
    if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
        return BOOL(1);
    }
    // app windows only: no tool windows, no owned popups
    let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 || GetWindow(h, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false) {
        return BOOL(1);
    }
    if GetAncestor(h, GA_ROOTOWNER) != h {
        return BOOL(1);
    }
    // UWP shells and other virtual-desktop windows are "cloaked"
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
    if cloaked != 0 {
        return BOOL(1);
    }
    let n = GetWindowTextLengthW(h);
    if n <= 0 {
        return BOOL(1);
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = GetWindowTextW(h, &mut buf);
    let title = String::from_utf16_lossy(&buf[..got.max(0) as usize]);
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    if let Some(px) = window_rect(h.0 as isize) {
        list.push(WinInfo { hwnd: h.0 as isize, title, app: String::new(), pid, px });
    }
    BOOL(1)
}

/// Visible, titled top-level application windows in z-order (front first).
pub fn windows() -> Vec<WinInfo> {
    let mut out: Vec<WinInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(window_cb), LPARAM(&mut out as *mut Vec<WinInfo> as isize));
    }
    for w in &mut out {
        w.app = process_name(w.pid);
    }
    out
}
