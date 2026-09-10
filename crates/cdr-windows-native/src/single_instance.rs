use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Threading::CreateMutexW;

use crate::error::{NativeError, api_error};
use crate::process::handle::OwnedHandle;

pub struct SingleInstance {
    _mutex: OwnedHandle,
}

impl SingleInstance {
    pub fn acquire(name: &str) -> Result<Self, NativeError> {
        if name.encode_utf16().any(|unit| unit == 0) {
            return Err(NativeError::InteriorNul("single-instance mutex name"));
        }
        let mut wide = name.encode_utf16().collect::<Vec<_>>();
        wide.push(0);
        // SAFETY: the security attributes pointer is null, the ownership flag is valid,
        // and `wide` is a live NUL-terminated UTF-16 buffer for the duration of the call.
        let raw = unsafe { CreateMutexW(ptr::null(), 0, wide.as_ptr()) };
        let mutex = OwnedHandle::new(raw).ok_or_else(|| api_error("CreateMutexW"))?;
        // SAFETY: GetLastError has no preconditions and is read immediately after CreateMutexW.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Err(NativeError::AlreadyRunning);
        }
        Ok(Self { _mutex: mutex })
    }
}
