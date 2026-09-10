use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};

use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};

use super::encoding::normalized_entries;
use crate::NativeError;

pub fn executable(
    program: &Path,
    environment: &HashMap<String, String>,
) -> Result<PathBuf, NativeError> {
    let name = program.as_os_str();
    if name.is_empty() || program.file_name().is_none() {
        return Err(NativeError::InvalidInput(
            "program path has no file name".into(),
        ));
    }
    if program.file_name() != Some(name) {
        return Ok(resolve_subpath(program));
    }
    let append_exe = !name.encode_wide().any(|unit| unit == u16::from(b'.'));
    for directory in search_directories(environment) {
        let mut candidate = directory.join(program);
        if append_exe {
            candidate.set_extension("exe");
        }
        if std::fs::symlink_metadata(&candidate).is_ok() {
            return std::path::absolute(&candidate).map_err(|error| {
                NativeError::InvalidInput(format!(
                    "could not make executable path absolute: {error}"
                ))
            });
        }
    }
    Err(NativeError::InvalidInput(format!(
        "program not found: {}",
        program.display()
    )))
}

fn resolve_subpath(program: &Path) -> PathBuf {
    if has_exe_suffix(program.as_os_str()) {
        return program.to_path_buf();
    }
    let mut with_exe = program.as_os_str().to_os_string();
    with_exe.push(".exe");
    let with_exe = PathBuf::from(with_exe);
    if std::fs::symlink_metadata(&with_exe).is_ok() {
        with_exe
    } else {
        program.to_path_buf()
    }
}

fn search_directories(environment: &HashMap<String, String>) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(path) = normalized_entries(environment)
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value)
    {
        directories.extend(nonempty_paths(OsStr::new(path)));
    }
    if let Ok(mut current_executable) = std::env::current_exe() {
        current_executable.pop();
        directories.push(current_executable);
    }
    if let Some(system) = windows_directory(GetSystemDirectoryW) {
        directories.push(system);
    }
    if let Some(windows) = windows_directory(GetWindowsDirectoryW) {
        directories.push(windows);
    }
    if let Some(parent_path) = std::env::var_os("PATH") {
        directories.extend(nonempty_paths(&parent_path));
    }
    directories
}

fn nonempty_paths(value: &OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(value).filter(|path| !path.as_os_str().is_empty())
}

type DirectoryFunction = unsafe extern "system" fn(*mut u16, u32) -> u32;

fn windows_directory(function: DirectoryFunction) -> Option<PathBuf> {
    let mut buffer = vec![0_u16; 260];
    loop {
        let capacity = u32::try_from(buffer.len()).ok()?;
        // SAFETY: `buffer` is writable for `capacity` UTF-16 code units.
        let length = unsafe { function(buffer.as_mut_ptr(), capacity) };
        if length == 0 {
            return None;
        }
        let length = usize::try_from(length).ok()?;
        if length < buffer.len() {
            return Some(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }
        buffer.resize(length.saturating_add(1), 0);
    }
}

fn has_exe_suffix(value: &OsStr) -> bool {
    let encoded = value.encode_wide().collect::<Vec<_>>();
    encoded.len() >= 4
        && encoded[encoded.len() - 4] == u16::from(b'.')
        && ascii_wide_eq(encoded[encoded.len() - 3], b'e')
        && ascii_wide_eq(encoded[encoded.len() - 2], b'x')
        && ascii_wide_eq(encoded[encoded.len() - 1], b'e')
}

fn ascii_wide_eq(unit: u16, expected: u8) -> bool {
    unit == u16::from(expected.to_ascii_lowercase())
        || unit == u16::from(expected.to_ascii_uppercase())
}
