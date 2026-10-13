//! Tauri commands, grouped by feature.

pub mod compress;
#[cfg_attr(windows, path = "../win/cmd_devices.rs")]
pub mod devices;
pub mod export;
pub mod files;
#[cfg_attr(windows, path = "../win/cmd_permissions.rs")]
pub mod permissions;
pub mod projects;
pub mod recording;
#[cfg_attr(windows, path = "../win/cmd_sources.rs")]
pub mod sources;
