//! `list_sources` on Windows: monitors and capturable windows, in display points.

use crate::state::CmdResult;
use crate::win::displays;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    id: u32,
    name: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    is_main: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    id: u32,
    title: String,
    app: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Serialize)]
pub struct Sources {
    displays: Vec<DisplayInfo>,
    windows: Vec<WindowInfo>,
}

#[tauri::command]
pub fn list_sources() -> CmdResult<Sources> {
    let me = std::process::id();
    let displays = displays::list()
        .into_iter()
        .enumerate()
        .map(|(i, d)| {
            let (x, y, width, height) = d.points();
            DisplayInfo {
                id: d.id,
                name: if d.primary { "Main Display".into() } else { format!("Display {}", i + 1) },
                x,
                y,
                width,
                height,
                is_main: d.primary,
            }
        })
        .collect();
    let windows = displays::windows()
        .into_iter()
        .filter(|w| w.pid != me && w.px.2 >= 120.0 && w.px.3 >= 80.0)
        .map(|w| {
            let s = displays::scale_at(w.px.0 + w.px.2 / 2.0, w.px.1 + w.px.3 / 2.0);
            WindowInfo {
                id: w.hwnd as u32,
                title: w.title,
                app: w.app,
                x: w.px.0 / s,
                y: w.px.1 / s,
                width: w.px.2 / s,
                height: w.px.3 / s,
            }
        })
        .collect();
    Ok(Sources { displays, windows })
}
