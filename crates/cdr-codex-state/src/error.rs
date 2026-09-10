use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodexStateError {
    #[error("Codex state database not found: {0}")]
    StateDatabaseMissing(PathBuf),
    #[error("Codex state SQLite failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Codex state file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Codex state JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Codex session walk failed: {0}")]
    Walk(#[from] walkdir::Error),
}
