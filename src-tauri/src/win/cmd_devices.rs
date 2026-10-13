//! Audio device listing (Media Foundation) and the live level meter on Windows.

use crate::state::{AppState, CmdResult};
use serde::Serialize;
use tauri::{AppHandle, State};
use windows::core::{GUID, PWSTR};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, MFCreateAttributes, MFEnumDeviceSources, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_AUDCAP_ENDPOINT_ID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_AUDCAP_GUID, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
};
use windows::Win32::System::Com::CoTaskMemFree;

#[derive(Serialize)]
pub struct Device {
    id: String,
    name: String,
}

unsafe fn string_attr(a: &IMFActivate, key: &GUID) -> String {
    let mut p = PWSTR::null();
    let mut len = 0u32;
    if a.GetAllocatedString(key, &mut p, &mut len).is_err() || p.is_null() {
        return String::new();
    }
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

fn devices(kind: &GUID, id_key: &GUID) -> Vec<Device> {
    crate::win::com_init();
    crate::win::mf::startup();
    unsafe {
        let mut attrs = None;
        if MFCreateAttributes(&mut attrs, 1).is_err() {
            return Vec::new();
        }
        let Some(attrs) = attrs else { return Vec::new() };
        if attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, kind).is_err() {
            return Vec::new();
        }
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        if MFEnumDeviceSources(&attrs, &mut list, &mut count).is_err() || list.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for i in 0..count as usize {
            if let Some(a) = (*list.add(i)).take() {
                out.push(Device { id: string_attr(&a, id_key), name: string_attr(&a, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME) });
            }
        }
        CoTaskMemFree(Some(list as *const _));
        out
    }
}

#[tauri::command]
pub fn list_audio_inputs() -> Vec<Device> {
    devices(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_AUDCAP_GUID, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_AUDCAP_ENDPOINT_ID)
}

#[tauri::command]
pub fn list_cameras() -> Vec<Device> {
    devices(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK)
}

#[tauri::command]
pub fn meter_start(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    state.meter.start(app)
}

#[tauri::command]
pub fn meter_stop(state: State<AppState>) {
    state.meter.stop();
}
