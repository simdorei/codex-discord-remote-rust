use crate::error::{NativeError, api_error};
use std::{
    fs::File,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;

/// Resolve the retained object, not a second lookup of its mutable input path.
/// Call inside the owned filesystem probe because normalization can perform I/O.
pub fn resolved_file_path(file: &File) -> Result<PathBuf, NativeError> {
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: the live File and bounded writable UTF-16 output remain valid.
    let count =
        unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), 32_768, 0) };
    if count == 0 {
        return Err(api_error("GetFinalPathNameByHandleW(lease)"));
    }
    if count >= 32_768 {
        return Err(NativeError::InvalidInput(
            "resolved artifact path too long".into(),
        ));
    }
    let value = String::from_utf16(&buffer[..count as usize])
        .map_err(|_| NativeError::InvalidInput("artifact path is not valid Unicode".into()))?;
    Ok(PathBuf::from(without_extended_prefix(&value)))
}

fn without_extended_prefix(value: &str) -> String {
    if let Some(unc) = value.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{unc}")
    } else {
        value.strip_prefix("\\\\?\\").unwrap_or(value).to_owned()
    }
}

/// Compare already resolved OS paths without starting any filesystem observation.
#[must_use]
pub fn same_resolved_path(left: &Path, right: &Path) -> bool {
    without_extended_prefix(&left.to_string_lossy())
        .replace('/', "\\")
        .eq_ignore_ascii_case(&without_extended_prefix(&right.to_string_lossy()).replace('/', "\\"))
}
