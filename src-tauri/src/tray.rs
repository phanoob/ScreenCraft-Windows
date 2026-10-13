//! The menu-bar / notification-area icon and its menu, in the UI language.

use crate::state::{save_settings, AppState};
use crate::windows;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, Wry};

/// (id, accelerator) in menu order.
const ITEMS: [(&str, Option<&str>); 5] = [
    ("new", Some("CmdOrCtrl+Shift+2")),
    ("compress", None),
    ("perms", None),
    ("stop", None),
    ("quit", Some("CmdOrCtrl+Q")),
];

fn label(id: &str, lang: &str) -> &'static str {
    match (id, lang) {
        ("new", "fr") => "Nouvel enregistrement",
        ("new", "de") => "Neue Aufnahme",
        ("new", _) => "New Recording",
        ("compress", "fr") => "Compresser une vidéo…",
        ("compress", "de") => "Video komprimieren…",
        ("compress", _) => "Compress Video…",
        ("perms", "fr") => "Autorisations…",
        ("perms", "de") => "Berechtigungen…",
        ("perms", _) => "Permissions…",
        ("stop", "fr") => "Arrêter l'enregistrement",
        ("stop", "de") => "Aufnahme beenden",
        ("stop", _) => "Stop Recording",
        ("quit", "fr") => "Quitter ScreenCraft",
        ("quit", "de") => "ScreenCraft beenden",
        _ => "Quit ScreenCraft",
    }
}

static MENU_ITEMS: Mutex<Vec<(&'static str, MenuItem<Wry>)>> = Mutex::new(Vec::new());

pub fn build(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let lang = app.state::<AppState>().settings.lock().unwrap().language.clone().unwrap_or_default();
    let mut items = Vec::new();
    for (id, accel) in ITEMS {
        items.push((id, MenuItem::with_id(app, id, label(id, &lang), true, accel)?));
    }
    let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|(_, m)| m as _).collect();
    let menu = Menu::with_items(app, &refs)?;
    *MENU_ITEMS.lock().unwrap() = items;
    TrayIconBuilder::with_id("tray")
        .icon(app.default_window_icon().cloned().ok_or("no icon")?)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "new" => {
                let _ = windows::show_recorder_window(app);
            }
            "perms" => {
                let _ = windows::show_permissions_window(app);
            }
            "compress" => {
                let _ = windows::show_compressor_window(app, None);
            }
            "stop" => {
                let _ = app.emit("stop-record", ());
            }
            "quit" => windows::request_quit(app),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// The UI picked a language ("en" | "fr" | "de"): remember it and relabel the menu.
#[tauri::command]
pub fn set_language(_app: AppHandle, state: State<AppState>, lang: String) {
    {
        let mut s = state.settings.lock().unwrap();
        if s.language.as_deref() != Some(lang.as_str()) {
            s.language = Some(lang.clone());
            save_settings(&s);
        }
    }
    for (id, item) in MENU_ITEMS.lock().unwrap().iter() {
        let _ = item.set_text(label(id, &lang));
    }
}
