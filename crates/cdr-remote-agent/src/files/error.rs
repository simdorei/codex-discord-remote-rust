use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RemoteFileError {
    #[error("{path}: {reason}")]
    UnsafePath { path: String, reason: String },
    #[error("{pattern}: {reason}")]
    UnsafePattern { pattern: String, reason: String },
    #[error("{path}: {reason}")]
    Size { path: String, reason: String },
    #[error("{path}: {reason}")]
    Encoding { path: String, reason: String },
    #[error("{path}: {reason}")]
    Conflict { path: String, reason: String },
    #[error("{pattern}: {reason}")]
    Limit { pattern: String, reason: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl RemoteFileError {
    pub(crate) fn unsafe_path(path: &Path, reason: impl Into<String>) -> Self {
        Self::UnsafePath {
            path: path.display().to_string(),
            reason: reason.into(),
        }
    }

    pub(crate) fn conflict(path: &Path, reason: impl Into<String>) -> Self {
        Self::Conflict {
            path: path.display().to_string(),
            reason: reason.into(),
        }
    }
}
