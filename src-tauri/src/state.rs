//! Shared application state and persisted settings.

use crate::capture::Capture;
use crate::meter::Meter;
use screencraft_core::input::InputLog;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Where new recordings are written. Defaults to ~/ScreenCraft Projects.
    pub projects_dir: Option<String>,
}

pub struct AppState {
    pub capture: Capture,
    pub meter: Meter,
    pub recording_dir: Mutex<Option<PathBuf>>,
    pub camera: Mutex<Option<(File, String)>>,
    pub mic: Mutex<Option<(File, String)>>,
    pub export: Mutex<Option<File>>,
    pub inputs: Mutex<HashMap<PathBuf, InputLog>>,
    pub settings: Mutex<Settings>,
}

pub type CmdResult<T> = Result<T, String>;

pub fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    PathBuf::from(std::env::var(var).unwrap_or_else(|_| ".".into()))
}

pub fn settings_path() -> PathBuf {
    #[cfg(windows)]
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("ScreenCraft").join("settings.json");
    }
    home().join("Library/Application Support/ScreenCraft/settings.json")
}

pub fn load_settings() -> Settings {
    std::fs::read(settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    let p = settings_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(s) {
        let _ = std::fs::write(p, json);
    }
}

impl AppState {
    pub fn projects_root(&self) -> PathBuf {
        self.settings
            .lock()
            .unwrap()
            .projects_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(default_projects_root)
    }
}

/// ~/ScreenCraft Projects on macOS; Videos\ScreenCraft on Windows.
#[cfg(not(windows))]
fn default_projects_root() -> PathBuf {
    home().join("ScreenCraft Projects")
}

#[cfg(windows)]
fn default_projects_root() -> PathBuf {
    videos_dir().join("ScreenCraft")
}

/// The user's Videos folder, wherever it lives (it can be moved, e.g. to OneDrive).
#[cfg(windows)]
pub fn videos_dir() -> PathBuf {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Videos, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Videos, KNOWN_FOLDER_FLAG(0), None) {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            if let Some(s) = s.filter(|s| !s.is_empty()) {
                return PathBuf::from(s);
            }
        }
    }
    home().join("Videos")
}

#[cfg(windows)]
pub fn timestamp() -> String {
    // local time; "." instead of ":" keeps it usable in file names
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}.{:02}.{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(not(windows))]
pub fn timestamp() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02} {:02}.{:02}.{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}
