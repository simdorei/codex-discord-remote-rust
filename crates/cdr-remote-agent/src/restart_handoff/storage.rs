use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::RestartHandoffError;

pub(super) fn write_atomic(path: &Path, content: &[u8]) -> Result<(), RestartHandoffError> {
    let parent = path.parent().ok_or(RestartHandoffError::Malformed)?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("handoff"),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        atomic_replace(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn claimed_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("handoff");
    path.with_file_name(format!(
        ".{name}.claimed.{}.{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

#[cfg(windows)]
pub(super) fn system_protect(payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
    cdr_windows_native::protect_current_user(payload)
        .map_err(|error| RestartHandoffError::Protection(error.to_string()))
}

#[cfg(not(windows))]
pub(super) fn system_protect(_payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
    Err(RestartHandoffError::UnsupportedPlatform)
}

#[cfg(windows)]
pub(super) fn system_unprotect(payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
    cdr_windows_native::unprotect_current_user(payload)
        .map_err(|error| RestartHandoffError::Protection(error.to_string()))
}

#[cfg(not(windows))]
pub(super) fn system_unprotect(_payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
    Err(RestartHandoffError::UnsupportedPlatform)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), RestartHandoffError> {
    cdr_windows_native::atomic_replace(source, destination)
        .map_err(|error| RestartHandoffError::Protection(error.to_string()))
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), RestartHandoffError> {
    fs::rename(source, destination).map_err(Into::into)
}
