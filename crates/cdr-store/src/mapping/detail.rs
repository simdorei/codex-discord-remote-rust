use std::path::Path;

use rusqlite::OptionalExtension;

use crate::schema::open_initialized;
use crate::{Result, StoreError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MirrorDetailMode {
    Send,
    All,
}

impl MirrorDetailMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::All => "all",
        }
    }
}

pub fn get_detail_mode(path: &Path, thread_id: &str) -> Result<MirrorDetailMode> {
    let mode = open_initialized(path)?
        .query_row(
            "SELECT detail_mode FROM session_mirror_details WHERE codex_thread_id = ?",
            [thread_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    match mode.as_deref() {
        None | Some("send") => Ok(MirrorDetailMode::Send),
        Some("all") => Ok(MirrorDetailMode::All),
        Some(other) => Err(StoreError::InvalidMirrorDetail(other.into())),
    }
}

pub fn set_detail_mode(path: &Path, thread_id: &str, mode: MirrorDetailMode) -> Result<()> {
    let connection = open_initialized(path)?;
    let mapped = connection
        .query_row(
            "SELECT 1 FROM mirror_threads WHERE codex_thread_id = ?",
            [thread_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some();
    if !mapped {
        return Err(StoreError::MirrorThreadNotFound(thread_id.into()));
    }
    connection.execute(
        "INSERT INTO session_mirror_details (codex_thread_id, detail_mode) VALUES (?, ?) \
         ON CONFLICT(codex_thread_id) DO UPDATE SET detail_mode = excluded.detail_mode",
        [thread_id, mode.as_str()],
    )?;
    Ok(())
}
