use crate::state::{save_settings, AppState, CmdResult};
use serde::Serialize;
use std::fs;
use tauri::State;

#[tauri::command]
pub fn reveal_in_finder(path: String) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer wants backslashes, and the /select, argument unquoted before the path
        let p = path.replace('/', "\\");
        let p = std::path::Path::new(&p);
        let _ = if p.is_dir() {
            std::process::Command::new("explorer").arg(p).spawn()
        } else {
            std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn()
        };
    }
}

#[tauri::command]
pub fn copy_file(src: String, dst: String) -> CmdResult<u64> {
    fs::copy(&src, &dst).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    projects_dir: String,
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> SettingsView {
    SettingsView { projects_dir: state.projects_root().to_string_lossy().into() }
}

#[tauri::command]
pub fn set_projects_dir(state: State<AppState>, path: Option<String>) -> CmdResult<String> {
    {
        let mut s = state.settings.lock().unwrap();
        s.projects_dir = path.filter(|p| !p.is_empty());
        save_settings(&s);
    }
    let root = state.projects_root();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root.to_string_lossy().into())
}

#[tauri::command]
pub fn remove_file(path: String) -> CmdResult<()> {
    fs::remove_file(&path).map_err(|e| e.to_string())
}

/// Where the export save sheet opens: Videos\ScreenCraft on Windows.
#[cfg(windows)]
#[tauri::command]
pub fn default_export_dir() -> String {
    let dir = crate::state::videos_dir().join("ScreenCraft");
    let _ = fs::create_dir_all(&dir);
    dir.to_string_lossy().into()
}

/// Where the export save sheet opens: ~/Movies when it exists, otherwise ~/Documents.
#[cfg(not(windows))]
#[tauri::command]
pub fn default_export_dir() -> String {
    let home = crate::state::home();
    let movies = home.join("Movies");
    if movies.is_dir() {
        movies.to_string_lossy().into()
    } else {
        home.join("Documents").to_string_lossy().into()
    }
}

/// Automation hook: when SCREENCRAFT_TEST_EXPORT_PATH is set the editor exports there
/// without showing the save sheet (used to test the export pipeline end to end).
#[tauri::command]
pub fn test_export_path() -> Option<String> {
    std::env::var("SCREENCRAFT_TEST_EXPORT_PATH").ok().filter(|p| !p.is_empty())
}

/// Read an image file as a `data:` URL (used for custom backgrounds).
#[tauri::command]
pub fn read_image_data_url(path: String) -> CmdResult<String> {
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    if bytes.len() > 40 * 1024 * 1024 {
        return Err("That image is larger than 40 MB".into());
    }
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    };
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 32);
    out.push_str("data:");
    out.push_str(mime);
    out.push_str(";base64,");
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if c.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if c.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    Ok(out)
}

/// Development aid: lets the page print timing lines to the app's stderr.
#[tauri::command]
pub fn dbg_log(msg: String) {
    eprintln!("[js] {msg}");
}
