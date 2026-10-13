//! Permissions on Windows. Screen capture, input monitoring and accessibility need
//! no grant; camera and microphone follow the Settings > Privacy switches, which
//! are read from the registry.

use serde::Serialize;
use tauri::AppHandle;
use windows::core::HSTRING;
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

fn consent(root: HKEY, capability: &str) -> Option<String> {
    let key = HSTRING::from(format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\{capability}"
    ));
    let mut buf = [0u16; 64];
    let mut len = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(root, &key, &HSTRING::from("Value"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
            .ok()
            .ok()?;
    }
    let n = (len as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// "granted" unless the user (or a policy) switched the capability off.
fn status(capability: &str) -> String {
    let denied = [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        .into_iter()
        .any(|root| consent(root, capability).map(|v| v.eq_ignore_ascii_case("Deny")).unwrap_or(false));
    if denied { "denied" } else { "granted" }.into()
}

/// Everything required before the recorder is usable.
pub fn all_required_granted() -> bool {
    true
}

#[derive(Serialize)]
pub struct PermissionStatus {
    screen: bool,
    camera: String,
    microphone: String,
    accessibility: bool,
    input_monitoring: bool,
    /// Granted *and* actually usable in this process (otherwise a restart is needed).
    screen_ready: bool,
    input_ready: bool,
}

#[tauri::command]
pub fn permission_status() -> PermissionStatus {
    PermissionStatus {
        screen: true,
        camera: status("webcam"),
        microphone: status("microphone"),
        accessibility: true,
        input_monitoring: true,
        screen_ready: windows_capture::graphics_capture_api::GraphicsCaptureApi::is_supported().unwrap_or(false),
        input_ready: true,
    }
}

#[tauri::command]
pub fn request_screen_access() -> bool {
    true
}

#[tauri::command]
pub fn request_input_monitoring() -> bool {
    true
}

#[tauri::command]
pub fn open_privacy_pane(kind: String) {
    let page = match kind.as_str() {
        "camera" => "ms-settings:privacy-webcam",
        "microphone" => "ms-settings:privacy-microphone",
        _ => "ms-settings:privacy",
    };
    let _ = std::process::Command::new("explorer").arg(page).spawn();
}

#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}
