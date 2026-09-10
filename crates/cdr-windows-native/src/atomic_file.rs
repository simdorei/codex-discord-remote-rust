use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

use crate::error::{NativeError, api_error};

pub fn atomic_replace(source: &Path, destination: &Path) -> Result<(), NativeError> {
    let source = wide_path(source);
    let destination = wide_path(destination);
    // SAFETY: Both paths are owned, NUL-terminated UTF-16 buffers that remain alive
    // for the duration of the call. MoveFileExW does not retain either pointer.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(api_error("MoveFileExW"));
    }
    Ok(())
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
