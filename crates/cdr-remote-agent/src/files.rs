mod atomic_write;
mod error;
pub(crate) mod internal;
mod listing;
mod path;
mod platform;
pub(crate) mod redaction;

use std::io::Read;
use std::path::Path;

use cdr_remote_protocol::message::{
    ListFilesOutput, ProjectInfoOutput, ReadFileOutput, WriteFileOutput,
};
use sha2::{Digest, Sha256};

pub use error::RemoteFileError;
use path::{relative_text, validate_relative};
use platform::RootGuard;

pub const MAX_FILE_BYTES: usize = 1_048_576;
pub const MAX_LIST_RESULTS: usize = 500;

pub struct ProjectFileAccess {
    guard: RootGuard,
}

impl ProjectFileAccess {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, RemoteFileError> {
        Ok(Self {
            guard: RootGuard::open(root.as_ref())?,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.guard.root()
    }

    pub fn verify_root(&self) -> Result<(), RemoteFileError> {
        self.guard.verify()
    }

    pub(crate) fn scan_paths(&self) -> Result<Vec<std::path::PathBuf>, RemoteFileError> {
        listing::scan_paths(&self.guard)
    }

    pub(crate) fn validate_path(value: &str) -> Result<String, RemoteFileError> {
        validate_relative(value).map(|path| relative_text(&path))
    }

    #[must_use]
    pub fn project_info(&self, thread_id: &str) -> ProjectInfoOutput {
        ProjectInfoOutput {
            root: self.root().display().to_string(),
            thread_id: thread_id.to_owned(),
        }
    }

    pub fn list_files(
        &self,
        pattern: &str,
        limit: u16,
    ) -> Result<ListFilesOutput, RemoteFileError> {
        listing::list_files(&self.guard, pattern, usize::from(limit))
    }

    pub fn read_file(
        &self,
        value: &str,
        start_line: u64,
        max_lines: u16,
    ) -> Result<ReadFileOutput, RemoteFileError> {
        let relative = validate_relative(value)?;
        let mut locked = self.guard.open_regular(&relative)?;
        let size = locked.file.metadata()?.len();
        if size > MAX_FILE_BYTES as u64 {
            return Err(RemoteFileError::Size {
                path: value.to_owned(),
                reason: format!("file exceeds {MAX_FILE_BYTES} bytes"),
            });
        }
        let capacity = usize::try_from(size).map_err(|_| RemoteFileError::Size {
            path: value.to_owned(),
            reason: format!("file exceeds {MAX_FILE_BYTES} bytes"),
        })?;
        let mut raw = Vec::with_capacity(capacity);
        locked
            .file
            .by_ref()
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut raw)?;
        if raw.len() > MAX_FILE_BYTES {
            return Err(RemoteFileError::Size {
                path: value.to_owned(),
                reason: format!("file exceeds {MAX_FILE_BYTES} bytes"),
            });
        }
        let text = std::str::from_utf8(&raw).map_err(|_| RemoteFileError::Encoding {
            path: value.to_owned(),
            reason: "file is not UTF-8 text".into(),
        })?;
        let lines = text.lines().collect::<Vec<_>>();
        let bounded_start = start_line.max(1);
        let bounded_count = usize::from(max_lines.clamp(1, 500));
        let start_index = usize::try_from(bounded_start - 1).unwrap_or(usize::MAX);
        let selected = lines
            .get(start_index..)
            .unwrap_or_default()
            .iter()
            .take(bounded_count)
            .copied()
            .collect::<Vec<_>>();
        let end_line = bounded_start - 1 + selected.len() as u64;
        let selected_content = selected.join("\n");
        let safe_content = redaction::redact(&selected_content);
        Ok(ReadFileOutput {
            path: relative_text(&relative),
            content: safe_content.clone(),
            sha256: hex_digest(&raw),
            start_line: bounded_start,
            end_line,
            total_lines: lines.len() as u64,
            truncated: end_line < lines.len() as u64,
            redacted: safe_content != selected_content,
        })
    }

    pub fn read_bytes(&self, value: &str, max_bytes: usize) -> Result<Vec<u8>, RemoteFileError> {
        let relative = validate_relative(value)?;
        let mut locked = self.guard.open_regular(&relative)?;
        if locked.file.metadata()?.len() > max_bytes as u64 {
            return Err(RemoteFileError::Size {
                path: value.to_owned(),
                reason: format!("file exceeds {max_bytes} bytes"),
            });
        }
        let mut raw = Vec::new();
        locked
            .file
            .by_ref()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut raw)?;
        if raw.len() > max_bytes {
            return Err(RemoteFileError::Size {
                path: value.to_owned(),
                reason: format!("file exceeds {max_bytes} bytes"),
            });
        }
        Ok(raw)
    }

    pub fn file_exists(&self, value: &str) -> Result<bool, RemoteFileError> {
        let relative = validate_relative(value)?;
        match self.guard.open_regular(&relative) {
            Ok(_) => Ok(true),
            Err(RemoteFileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(false)
            }
            Err(RemoteFileError::UnsafePath { reason, .. })
                if reason == "path is not a regular file" =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    pub fn write_file(
        &self,
        value: &str,
        content: &str,
        expected_sha256: Option<&str>,
    ) -> Result<WriteFileOutput, RemoteFileError> {
        let raw = content.as_bytes();
        let created = self.write_bytes(value, raw, MAX_FILE_BYTES, expected_sha256)?;
        let relative = validate_relative(value)?;
        Ok(WriteFileOutput {
            path: relative_text(&relative),
            sha256: hex_digest(raw),
            bytes_written: raw.len() as u64,
            created,
        })
    }

    pub fn write_bytes(
        &self,
        value: &str,
        content: &[u8],
        max_bytes: usize,
        expected_sha256: Option<&str>,
    ) -> Result<bool, RemoteFileError> {
        let relative = validate_relative(value)?;
        if content.len() > max_bytes {
            return Err(RemoteFileError::Size {
                path: value.to_owned(),
                reason: format!("content exceeds {max_bytes} bytes"),
            });
        }
        atomic_write::write(&self.guard, &relative, content, expected_sha256)
    }

    pub fn delete_file(&self, value: &str, expected_sha256: &str) -> Result<(), RemoteFileError> {
        let relative = validate_relative(value)?;
        atomic_write::delete(&self.guard, &relative, expected_sha256)
    }
}

pub(crate) fn hex_digest(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}
