use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsWindow, IsWindowVisible, SW_RESTORE,
    SetForegroundWindow, ShowWindow,
};

use crate::error::{NativeError, api_error};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowRect {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowInfo {
    pub window_id: u64,
    pub process_id: u32,
    pub title: String,
    pub rect: WindowRect,
}

pub fn find_window_by_title_suffix(
    suffix: &str,
    timeout: Duration,
) -> Result<WindowInfo, NativeError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(window) = find_once(suffix)? {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(NativeError::WindowOpenTimeout);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn inspect_window(
    window_id: u64,
    expected_process_id: Option<u32>,
) -> Result<Option<WindowInfo>, NativeError> {
    let hwnd = hwnd(window_id);
    // SAFETY: querying whether an integer-derived HWND currently names a live window is valid.
    if unsafe { IsWindow(hwnd) } == 0 {
        return Ok(None);
    }
    let mut process_id = 0;
    // SAFETY: `process_id` is a valid output pointer and HWND liveness was checked above.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &raw mut process_id) };
    if thread_id == 0 || process_id == 0 {
        return Err(api_error("GetWindowThreadProcessId"));
    }
    if expected_process_id.is_some_and(|expected| expected != process_id) {
        return Err(NativeError::IdentityChanged);
    }
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid output buffer and HWND liveness was checked.
    if unsafe { GetWindowRect(hwnd, &raw mut rect) } == 0 {
        return Err(api_error("GetWindowRect"));
    }
    let width = u32::try_from(rect.right - rect.left).map_err(|_| NativeError::InvalidBounds)?;
    let height = u32::try_from(rect.bottom - rect.top).map_err(|_| NativeError::InvalidBounds)?;
    if width == 0 || height == 0 {
        return Err(NativeError::InvalidBounds);
    }
    Ok(Some(WindowInfo {
        window_id,
        process_id,
        title: window_title(hwnd)?,
        rect: WindowRect {
            left: rect.left,
            top: rect.top,
            width,
            height,
        },
    }))
}

pub fn activate_window(window_id: u64, expected_process_id: u32) -> Result<bool, NativeError> {
    let _ =
        inspect_window(window_id, Some(expected_process_id))?.ok_or(NativeError::WindowMissing)?;
    let target = hwnd(window_id);
    // SAFETY: the target HWND and its owning process were just revalidated.
    let _ = unsafe { ShowWindow(target, SW_RESTORE) };
    for _ in 0..4 {
        // SAFETY: GetForegroundWindow has no pointer preconditions.
        if unsafe { GetForegroundWindow() } == target {
            return Ok(true);
        }
        set_foreground(target)?;
        std::thread::sleep(Duration::from_millis(120));
    }
    Err(NativeError::InvalidInput(
        "Windows did not activate the terminal window".into(),
    ))
}

pub(super) fn require_active(window_id: u64, process_id: u32) -> Result<(), NativeError> {
    let _ = inspect_window(window_id, Some(process_id))?.ok_or(NativeError::WindowMissing)?;
    // SAFETY: GetForegroundWindow has no pointer preconditions.
    if unsafe { GetForegroundWindow() } == hwnd(window_id) {
        Ok(())
    } else {
        Err(NativeError::InvalidInput(
            "active window changed during terminal input".into(),
        ))
    }
}

fn find_once(suffix: &str) -> Result<Option<WindowInfo>, NativeError> {
    let mut context = SearchContext {
        suffix,
        found: None,
    };
    // SAFETY: callback receives a pointer to `context`, which remains live for this synchronous call.
    let result = unsafe {
        EnumWindows(
            Some(visit_window),
            (&raw mut context).cast::<core::ffi::c_void>() as LPARAM,
        )
    };
    if result == 0 && context.found.is_none() {
        return Err(api_error("EnumWindows"));
    }
    context
        .found
        .map_or(Ok(None), |id| inspect_window(id, None))
}

struct SearchContext<'a> {
    suffix: &'a str,
    found: Option<u64>,
}

unsafe extern "system" fn visit_window(window: HWND, parameter: LPARAM) -> i32 {
    // SAFETY: EnumWindows passes back the exact non-null SearchContext pointer supplied by find_once.
    let context = unsafe { &mut *(parameter as *mut SearchContext<'_>) };
    // SAFETY: EnumWindows supplies a valid HWND for the callback duration.
    if unsafe { IsWindowVisible(window) } != 0
        && window_title(window).is_ok_and(|title| title.trim_end().ends_with(context.suffix))
    {
        context.found = Some(window as usize as u64);
        return 0;
    }
    1
}

fn window_title(window: HWND) -> Result<String, NativeError> {
    // SAFETY: querying the title length of an HWND supplied or revalidated by Windows is valid.
    let length = unsafe { GetWindowTextLengthW(window) };
    if length <= 0 {
        return Ok(String::new());
    }
    let mut buffer = vec![0_u16; usize::try_from(length).unwrap_or_default() + 1];
    // SAFETY: the buffer is writable for the reported length plus terminator.
    let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), length + 1) };
    if copied <= 0 {
        return Err(api_error("GetWindowTextW"));
    }
    Ok(String::from_utf16_lossy(
        &buffer[..usize::try_from(copied).unwrap_or_default()],
    ))
}

fn set_foreground(target: HWND) -> Result<(), NativeError> {
    // SAFETY: all calls receive HWNDs/thread IDs returned by Windows in this function.
    unsafe {
        if SetForegroundWindow(target) != 0 {
            return Ok(());
        }
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, std::ptr::null_mut());
        let target_thread = GetWindowThreadProcessId(target, std::ptr::null_mut());
        let current_thread = GetCurrentThreadId();
        let mut attached = Vec::new();
        for thread in [foreground_thread, target_thread] {
            if thread != 0
                && thread != current_thread
                && AttachThreadInput(current_thread, thread, 1) != 0
            {
                attached.push(thread);
            }
        }
        let _ = BringWindowToTop(target);
        let result = SetForegroundWindow(target);
        for thread in attached.into_iter().rev() {
            let _ = AttachThreadInput(current_thread, thread, 0);
        }
        if result == 0 {
            Err(api_error("SetForegroundWindow"))
        } else {
            Ok(())
        }
    }
}

fn hwnd(window_id: u64) -> HWND {
    usize::try_from(window_id).unwrap_or_default() as HWND
}
