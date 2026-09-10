use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, IsWindowVisible,
};

use super::identity::{WindowRect, inspect_window};
use crate::error::{NativeError, api_error};
use crate::process::handle::OwnedHandle;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceWindowInfo {
    pub window_id: u64,
    pub process_id: u32,
    pub process_path: String,
    pub title: String,
    pub rect: WindowRect,
    pub active: bool,
}

pub fn enumerate_visible_windows() -> Result<Vec<DeviceWindowInfo>, NativeError> {
    let mut windows = Vec::new();
    // SAFETY: callback receives the exact live Vec pointer for this synchronous enumeration.
    let result = unsafe {
        EnumWindows(
            Some(visit_window),
            (&raw mut windows).cast::<core::ffi::c_void>() as LPARAM,
        )
    };
    if result == 0 {
        Err(api_error("EnumWindows"))
    } else {
        Ok(windows)
    }
}

pub fn inspect_visible_window(window_id: u64) -> Result<DeviceWindowInfo, NativeError> {
    let hwnd = hwnd(window_id);
    // SAFETY: querying visibility for an integer-derived HWND is valid.
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return Err(NativeError::WindowMissing);
    }
    let window = inspect_window(window_id, None)?.ok_or(NativeError::WindowMissing)?;
    if window.title.trim().is_empty() {
        return Err(NativeError::InvalidInput(
            "the selected window has no usable title".into(),
        ));
    }
    // SAFETY: GetForegroundWindow has no pointer preconditions.
    let active = unsafe { GetForegroundWindow() } == hwnd;
    Ok(DeviceWindowInfo {
        window_id,
        process_id: window.process_id,
        process_path: process_path(window.process_id)?,
        title: window.title,
        rect: window.rect,
        active,
    })
}

unsafe extern "system" fn visit_window(window: HWND, parameter: LPARAM) -> i32 {
    // SAFETY: EnumWindows returns the exact non-null Vec pointer supplied above.
    let windows = unsafe { &mut *(parameter as *mut Vec<DeviceWindowInfo>) };
    let window_id = window as usize as u64;
    if let Ok(info) = inspect_visible_window(window_id)
        && info.rect.width >= 80
        && info.rect.height >= 60
    {
        windows.push(info);
    }
    1
}

fn process_path(process_id: u32) -> Result<String, NativeError> {
    // SAFETY: opening a process by an ID returned by GetWindowThreadProcessId is valid.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    let handle = OwnedHandle::new(raw).ok_or_else(|| api_error("OpenProcess"))?;
    let mut buffer = vec![0_u16; 32_768];
    let mut size = u32::try_from(buffer.len()).expect("bounded path buffer");
    // SAFETY: handle is live; buffer and size are writable and correctly sized.
    if unsafe { QueryFullProcessImageNameW(handle.raw(), 0, buffer.as_mut_ptr(), &raw mut size) }
        == 0
    {
        return Err(api_error("QueryFullProcessImageNameW"));
    }
    let path = OsString::from_wide(&buffer[..usize::try_from(size).unwrap_or_default()]);
    Ok(path.to_string_lossy().into_owned())
}

fn hwnd(window_id: u64) -> HWND {
    usize::try_from(window_id).unwrap_or_default() as HWND
}
