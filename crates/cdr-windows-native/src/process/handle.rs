use std::fs::File;
use std::mem::ManuallyDrop;
use std::os::windows::io::FromRawHandle as _;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

pub struct OwnedHandle(isize);

impl OwnedHandle {
    pub fn new(raw: HANDLE) -> Option<Self> {
        (!raw.is_null()).then(|| Self(raw as isize))
    }

    pub fn new_file(raw: HANDLE) -> Option<Self> {
        (!raw.is_null() && raw != INVALID_HANDLE_VALUE).then(|| Self(raw as isize))
    }

    pub fn raw(&self) -> HANDLE {
        self.0 as HANDLE
    }

    pub fn into_file(self) -> File {
        let owned = ManuallyDrop::new(self);
        // SAFETY: ownership of this valid Win32 file handle moves into `File` exactly once.
        unsafe { File::from_raw_handle(owned.raw()) }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this type is the sole owner of the non-null Win32 handle.
        let _ = unsafe { CloseHandle(self.raw()) };
    }
}
