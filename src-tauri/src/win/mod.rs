//! Windows back end. The modules that replace a macOS file one-for-one (capture,
//! proxy decoding, compression, …) are swapped in from `main.rs` with `#[path]`;
//! the helpers they share live here.

pub mod displays;
pub mod loopback;
pub mod mf;
pub mod strip;

use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

/// Join the multithreaded COM apartment on the current thread (idempotent).
pub fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// Let our own pages use the camera and microphone without WebView2's permission
/// bubble (on macOS the system's privacy prompt covers this; on Windows the
/// Settings > Privacy switches still apply on top).
pub fn allow_media(win: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
        COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    };
    use webview2_com::PermissionRequestedEventHandler;
    let _ = win.with_webview(|wv| unsafe {
        let Ok(core) = wv.controller().CoreWebView2() else { return };
        let handler = PermissionRequestedEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            args.PermissionKind(&mut kind)?;
            if kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA || kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
            }
            Ok(())
        }));
        let mut token = 0i64;
        let _ = core.add_PermissionRequested(&handler, &mut token);
    });
}

/// Keep one of our windows out of every screen capture (ours and other apps'),
/// like ScreenCaptureKit's "exclude own windows" filter on macOS.
pub fn exclude_from_capture(win: &tauri::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};
    if let Ok(hwnd) = win.hwnd() {
        unsafe {
            let _ = SetWindowDisplayAffinity(windows::Win32::Foundation::HWND(hwnd.0 as _), WDA_EXCLUDEFROMCAPTURE);
        }
    }
}
