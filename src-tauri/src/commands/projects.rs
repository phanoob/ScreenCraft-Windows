use crate::state::{AppState, CmdResult};
use screencraft_core::input::InputLog;
use screencraft_core::project::{Project, ZoomRegion};
use screencraft_core::track::{compute_track, RenderTrack};
use screencraft_core::zoom::{generate, AutoZoomOptions};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

pub fn write_project(dir: &Path, project: &Project) -> CmdResult<()> {
    let json = serde_json::to_vec_pretty(project).map_err(|e| e.to_string())?;
    fs::write(dir.join("project.json"), json).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct Loaded {
    project: Project,
    input: InputLog,
    dir: String,
}

pub fn read_input(state: &AppState, dir: &Path, rel: &str) -> InputLog {
    let key = dir.to_path_buf();
    if let Some(l) = state.inputs.lock().unwrap().get(&key) {
        return l.clone();
    }
    let mut log: InputLog = fs::read(dir.join(rel))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    log.refine_carets();
    state.inputs.lock().unwrap().insert(key, log.clone());
    log
}

#[tauri::command]
pub fn load_project(state: State<AppState>, dir: String) -> CmdResult<Loaded> {
    let d = PathBuf::from(&dir);
    let bytes = fs::read(d.join("project.json")).map_err(|e| format!("cannot read project: {e}"))?;
    let project: Project = serde_json::from_slice(&bytes).map_err(|e| format!("invalid project: {e}"))?;
    let input = read_input(&state, &d, &project.media.input);
    Ok(Loaded { project, input, dir })
}

#[tauri::command]
pub fn save_project(dir: String, project: Project) -> CmdResult<()> {
    write_project(Path::new(&dir), &project)
}

#[tauri::command]
pub fn compute_render_track(state: State<AppState>, dir: String, project: Project, fps: f64) -> CmdResult<RenderTrack> {
    let log = read_input(&state, Path::new(&dir), &project.media.input);
    Ok(compute_track(&project, &log, fps.clamp(1.0, 240.0)))
}

#[tauri::command]
pub fn auto_zoom(state: State<AppState>, dir: String, project: Project, zoom: f64) -> CmdResult<Vec<ZoomRegion>> {
    let log = read_input(&state, Path::new(&dir), &project.media.input);
    let opts = AutoZoomOptions::for_ease(zoom, project.config.animations.ease_in_ms);
    Ok(generate(&log, project.media.duration_ms, &opts))
}

#[derive(Serialize)]
pub struct RecentProject {
    dir: String,
    name: String,
    modified: u64,
}

#[tauri::command]
pub fn recent_projects(state: State<AppState>) -> Vec<RecentProject> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(state.projects_root()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "screencraft").unwrap_or(false) && p.join("project.json").exists() {
                let modified = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                out.push(RecentProject {
                    dir: p.to_string_lossy().into(),
                    name: p.file_stem().map(|s| s.to_string_lossy().into()).unwrap_or_default(),
                    modified,
                });
            }
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

#[tauri::command]
pub fn projects_dir(state: State<AppState>) -> String {
    state.projects_root().to_string_lossy().into()
}

/// Rename a project: moves the package folder to `<name>.screencraft` and updates the
/// stored name. Returns the new folder path.
#[tauri::command]
pub fn rename_project(state: State<AppState>, dir: String, name: String) -> CmdResult<String> {
    let clean: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '\0' | '<' | '>' | '"' | '|' | '?' | '*') || c.is_control() { '-' } else { c })
        .collect();
    let clean = clean.trim().trim_start_matches('.').trim().to_string();
    if clean.is_empty() {
        return Err("Name can't be empty".into());
    }
    let old = PathBuf::from(&dir);
    let parent = old.parent().ok_or("Bad project path")?.to_path_buf();
    let new = parent.join(format!("{clean}.screencraft"));
    if new != old {
        if new.exists() {
            return Err(format!("A project named “{clean}” already exists"));
        }
        fs::rename(&old, &new).map_err(|e| e.to_string())?;
    }
    let mut project: Project = serde_json::from_slice(&fs::read(new.join("project.json")).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    project.name = clean;
    write_project(&new, &project)?;
    let _ = &state;
    Ok(new.to_string_lossy().into())
}
