use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL,
    mouse_event,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, SetCursorPos, WM_CLOSE};

use super::identity::{inspect_window, require_active};
use crate::error::{NativeError, api_error};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMouseButton {
    Left,
    Right,
    Middle,
}

pub fn click_at(
    window_id: u64,
    process_id: u32,
    x: i32,
    y: i32,
    button: NativeMouseButton,
    count: u8,
) -> Result<(), NativeError> {
    require_active(window_id, process_id)?;
    set_cursor(x, y)?;
    let (down, up) = button_flags(button);
    for _ in 0..count {
        // SAFETY: mouse_event accepts these documented button flags without pointers.
        unsafe {
            mouse_event(down, 0, 0, 0, 0);
            mouse_event(up, 0, 0, 0, 0);
        }
    }
    require_active(window_id, process_id)
}

pub fn drag_at(
    window_id: u64,
    process_id: u32,
    start: (i32, i32),
    end: (i32, i32),
) -> Result<(), NativeError> {
    require_active(window_id, process_id)?;
    set_cursor(start.0, start.1)?;
    // SAFETY: paired documented left-button flags require no pointer arguments.
    unsafe { mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0) };
    let moved = set_cursor(end.0, end.1);
    // SAFETY: always releases the button even if moving the cursor failed.
    unsafe { mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0) };
    moved?;
    require_active(window_id, process_id)
}

pub fn scroll_at(
    window_id: u64,
    process_id: u32,
    point: (i32, i32),
    delta: (i32, i32),
) -> Result<(), NativeError> {
    require_active(window_id, process_id)?;
    set_cursor(point.0, point.1)?;
    // SAFETY: wheel flags and signed deltas are passed by value without pointers.
    unsafe {
        if delta.1 != 0 {
            mouse_event(MOUSEEVENTF_WHEEL, 0, 0, delta.1, 0);
        }
        if delta.0 != 0 {
            mouse_event(MOUSEEVENTF_HWHEEL, 0, 0, delta.0, 0);
        }
    }
    require_active(window_id, process_id)
}

pub fn post_close(window_id: u64, process_id: u32) -> Result<(), NativeError> {
    let _ = inspect_window(window_id, Some(process_id))?.ok_or(NativeError::WindowMissing)?;
    let hwnd = usize::try_from(window_id).unwrap_or_default() as *mut core::ffi::c_void;
    // SAFETY: the HWND and owning process were just revalidated; arguments contain no pointers.
    if unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM::default(), LPARAM::default()) } == 0 {
        Err(api_error("PostMessageW(WM_CLOSE)"))
    } else {
        Ok(())
    }
}

fn button_flags(button: NativeMouseButton) -> (u32, u32) {
    match button {
        NativeMouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        NativeMouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        NativeMouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    }
}

fn set_cursor(x: i32, y: i32) -> Result<(), NativeError> {
    // SAFETY: SetCursorPos accepts screen coordinates by value.
    if unsafe { SetCursorPos(x, y) } == 0 {
        Err(api_error("SetCursorPos"))
    } else {
        Ok(())
    }
}
