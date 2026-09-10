use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::atomic_write;
use super::platform::{RootGuard, is_reparse};
use super::{RemoteFileError, hex_digest};

const CHECKPOINT_DIRECTORY: &str = ".codex-remote-mcp/checkpoints";

pub struct InternalEntry {
    pub name: String,
    pub modified: SystemTime,
    pub size: u64,
}

pub struct CheckpointStore {
    guard: RootGuard,
}

impl CheckpointStore {
    pub fn open(root: &Path) -> Result<Self, RemoteFileError> {
        Ok(Self {
            guard: RootGuard::open(root)?,
        })
    }

    pub fn write_new(&self, name: &str, content: &[u8]) -> Result<(), RemoteFileError> {
        atomic_write::write(&self.guard, &relative(name), content, None).map(|_| ())
    }

    pub fn read(&self, name: &str, max_bytes: usize) -> Result<Vec<u8>, RemoteFileError> {
        let mut locked = self.guard.open_regular(&relative(name))?;
        if locked.file.metadata()?.len() > max_bytes as u64 {
            return Err(RemoteFileError::Size {
                path: name.to_owned(),
                reason: format!("checkpoint exceeds {max_bytes} bytes"),
            });
        }
        let mut content = Vec::new();
        locked
            .file
            .by_ref()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut content)?;
        if content.len() > max_bytes {
            return Err(RemoteFileError::Size {
                path: name.to_owned(),
                reason: format!("checkpoint exceeds {max_bytes} bytes"),
            });
        }
        Ok(content)
    }

    pub fn entries(&self) -> Result<Vec<InternalEntry>, RemoteFileError> {
        let directory = Path::new(CHECKPOINT_DIRECTORY);
        let (entries, _retained) = match self.guard.read_directory(directory) {
            Ok(value) => value,
            Err(RemoteFileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        let mut output = Vec::new();
        for entry in entries {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() || is_reparse(&metadata) {
                continue;
            }
            output.push(InternalEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                modified: metadata.modified()?,
                size: metadata.len(),
            });
        }
        Ok(output)
    }

    pub fn remove(&self, name: &str) -> Result<(), RemoteFileError> {
        let content = self.read(name, usize::MAX)?;
        atomic_write::delete(&self.guard, &relative(name), &hex_digest(&content))
    }
}

fn relative(name: &str) -> PathBuf {
    Path::new(CHECKPOINT_DIRECTORY).join(name)
}
