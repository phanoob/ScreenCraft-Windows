#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Platform back ends: each `win/` file replaces its macOS namesake one-for-one.
#[cfg_attr(windows, path = "win/ax.rs")]
mod ax;
#[cfg_attr(windows, path = "win/capture.rs")]
mod capture;
mod commands;
#[cfg_attr(windows, path = "win/compress.rs")]
mod compress;
#[cfg_attr(windows, path = "win/inputlog.rs")]
mod inputlog;
#[cfg_attr(windows, path = "win/jpegio.rs")]
mod jpegio;
mod levelmic;
mod proxy;
#[cfg_attr(windows, path = "win/srcframes.rs")]
mod srcframes;
#[cfg_attr(windows, path = "win/meter.rs")]
mod meter;
mod state;
mod tray;
#[cfg(windows)]
mod win;
mod windows;

use commands::permissions::all_required_granted;
use state::{load_settings, AppState};
use std::fs;
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

fn main() {
    // ⌘⇧2 on macOS; Ctrl+Shift+2 on Windows (Win+Shift+digit belongs to the taskbar)
    let primary = if cfg!(target_os = "macos") { Modifiers::SUPER } else { Modifiers::CONTROL };
    let toggle = Shortcut::new(Some(primary | Modifiers::SHIFT), Code::Digit2);
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if shortcut == &toggle && event.state() == ShortcutState::Pressed {
                        let _ = app.emit("toggle-record", ());
                    }
                })
                .build(),
        )
        .register_asynchronous_uri_scheme_protocol("sc", |_ctx, req, responder| {
            // /frame/<screen|camera>/<n>?d=<dir>        proxy frame (preview)
            // /src/<screen|camera>?t=<ms>&d=<dir>&q=<1-100>   full-res source frame (export)
            let uri = req.uri().to_string();
            let path = req.uri().path().to_string();
            let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
            let query = uri.split('?').nth(1).unwrap_or("").to_string();
            let param = |k: &str| -> String {
                query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix(&format!("{k}=")))
                    .map(urlencoding_decode)
                    .unwrap_or_default()
            };
            let respond = move |body: Option<Vec<u8>>| {
                let b = tauri::http::Response::builder()
                    .header("Access-Control-Allow-Origin", "*")
                    .header("Cache-Control", "no-store");
                let r = match body {
                    Some(bytes) => b.header("Content-Type", "image/jpeg").body(bytes).unwrap(),
                    None => b.status(404).body(Vec::new()).unwrap(),
                };
                responder.respond(r);
            };
            let dir = param("d");
            if parts.len() == 3 && parts[0] == "frame" {
                let n = parts[2].parse::<usize>().ok();
                respond(n.and_then(|n| proxy::frame_bytes(&dir, parts[1], n)));
            } else if parts.len() == 2 && parts[0] == "src" {
                let t_ms = param("t").parse::<f64>().unwrap_or(0.0);
                let quality = param("q").parse::<u8>().unwrap_or(90).clamp(30, 100);
                srcframes::submit(srcframes::Request {
                    dir,
                    kind: parts[1].to_string(),
                    t_ms,
                    quality,
                    reply: Box::new(respond),
                });
            } else {
                respond(None);
            }
        });
    // The app menu lives in the macOS menu bar; on Windows it would become a menu
    // strip inside every window, so it is left out (the tray menu has Quit).
    #[cfg(target_os = "macos")]
    let builder = builder
        .menu(|handle| {
            use tauri::menu::{Menu, MenuItem, MenuItemKind};
            let menu = Menu::default(handle)?;
            // Replace the stock Quit item with our own so open editors can ask about unsaved edits.
            if let Ok(items) = menu.items() {
                if let Some(MenuItemKind::Submenu(app_menu)) = items.into_iter().next() {
                    if let Ok(sub) = app_menu.items() {
                        if let Some(last) = sub.into_iter().last() {
                            let _ = app_menu.remove(&last);
                        }
                    }
                    let quit = MenuItem::with_id(handle, "sc-quit", "Quit ScreenCraft", true, Some("CmdOrCtrl+Q"))?;
                    app_menu.append(&quit)?;
                }
            }
            Ok(menu)
        })
        .on_menu_event(|app, event| {
            if event.id() == "sc-quit" {
                windows::request_quit(app);
            }
        });
    builder
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                windows::on_window_destroyed(window.app_handle(), window.label());
            }
        })
        .manage(AppState {
            capture: capture::Capture::spawn(),
            meter: Default::default(),
            recording_dir: Mutex::new(None),
            camera: Mutex::new(None),
            mic: Mutex::new(None),
            export: Mutex::new(None),
            inputs: Mutex::new(Default::default()),
            settings: Mutex::new(load_settings()),
        })
        .invoke_handler(tauri::generate_handler![
            commands::sources::list_sources,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::recording::camera_begin,
            commands::recording::camera_chunk,
            commands::recording::mic_begin,
            commands::recording::mic_chunk,
            commands::projects::load_project,
            commands::projects::save_project,
            windows::cancel_quit,
            levelmic::level_mic,
            proxy::ensure_proxy,
            windows::resize_bar,
            windows::open_menu_window,
            windows::menu_take_items,
            windows::menu_show,
            windows::close_menu_window,
            commands::files::read_image_data_url,
            commands::files::dbg_log,
            windows::close_area_picker,
            commands::projects::rename_project,
            commands::projects::compute_render_track,
            commands::projects::auto_zoom,
            commands::projects::recent_projects,
            commands::projects::projects_dir,
            commands::export::export_begin,
            commands::export::export_write,
            commands::export::export_end,
            commands::files::reveal_in_finder,
            commands::files::copy_file,
            commands::files::remove_file,
            commands::files::default_export_dir,
            commands::files::test_export_path,
            commands::files::get_settings,
            commands::files::set_projects_dir,
            commands::compress::probe_video,
            commands::compress::compress_video,
            commands::compress::cancel_compress,
            commands::compress::estimate_bitrate,
            commands::permissions::permission_status,
            commands::permissions::request_screen_access,
            commands::permissions::request_input_monitoring,
            commands::permissions::open_privacy_pane,
            commands::permissions::restart_app,
            commands::devices::list_audio_inputs,
            commands::devices::list_cameras,
            commands::devices::meter_start,
            commands::devices::meter_stop,
            windows::show_recorder,
            windows::show_permissions,
            windows::show_compressor,
            windows::open_editor,
            windows::open_area_picker,
            windows::show_area_frame,
            windows::hide_area_frame,
            windows::quit_app,
            tray::set_language,
        ])
        .setup(move |app| {
            let _ = app.global_shortcut().register(toggle);
            tray::build(app)?;
            let state = app.state::<AppState>();
            fs::create_dir_all(state.projects_root()).ok();
            if all_required_granted() {
                windows::show_recorder_window(app.handle())?;
            } else {
                windows::show_permissions_window(app.handle())?;
            }
            windows::prewarm_menu_window(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building ScreenCraft")
        .run(|app, event| {
            // ⌘Q and friends: give open editors the chance to ask about unsaved edits
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !windows::ALLOW_EXIT.load(std::sync::atomic::Ordering::SeqCst) {
                    api.prevent_exit();
                    windows::request_quit(app);
                }
            }
        });
}

fn urlencoding_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
