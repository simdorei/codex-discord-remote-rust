use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};
use thiserror::Error;
mod backup;
mod metadata_write;
mod records;
use records::{delete_log_rows, delete_state_row, verify_deleted};
#[cfg(test)]
mod guard_tests;
mod scrub;
mod transcript;
mod verification;

use backup::{backup_inputs, create_backup_dir};
use scrub::{scrub_json_state, scrub_session_index};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveDeletePaths {
    pub state_db: PathBuf,
    pub log_db: PathBuf,
    pub global_state: PathBuf,
    pub bridge_state: PathBuf,
    pub session_index: PathBuf,
    pub archived_sessions: PathBuf,
    pub backup_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveDeleteResult {
    pub backup_dir: PathBuf,
    pub backup_paths: Vec<PathBuf>,
    pub deleted_log_rows: usize,
    pub deleted_rollout_path: PathBuf,
    pub bridge_state_scrubbed: Vec<String>,
    pub global_state_scrubbed: Vec<String>,
    pub session_index_removed: usize,
}

#[derive(Debug, Error)]
pub enum ArchiveDeleteError {
    #[error(
        "archive deletion stopped; last completed stage: {last_completed_stage}; backup: {backup_dir}. Do not retry automatically; reconcile the original failure and restore only verified affected records/files from this backup: {source}"
    )]
    Partial {
        last_completed_stage: &'static str,
        backup_dir: PathBuf,
        source: Box<ArchiveDeleteError>,
    },
    #[error("archived thread does not exist in the local state database: {0}")]
    MissingThread(String),
    #[error("refusing to delete an active thread; only archived threads can be deleted")]
    ActiveThread,
    #[error("refusing to delete a rollout path outside the archived_sessions directory")]
    OutsideArchive,
    #[error("archived thread changed concurrently; deletion was cancelled")]
    ConcurrentMutation,
    #[error("archived thread row remains after deletion")]
    ThreadStillPresent,
    #[error("archived thread logs remain after deletion")]
    LogsStillPresent,
    #[error("archived rollout file remains after deletion")]
    RolloutStillPresent,
    #[error("SQLite archive deletion operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not access archive deletion path {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("archive deletion JSON file {path} is invalid: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

pub fn delete_archived_thread(
    paths: &ArchiveDeletePaths,
    thread_id: &str,
) -> Result<ArchiveDeleteResult, ArchiveDeleteError> {
    delete_with_metadata_observer(paths, thread_id, |_| {})
}

fn delete_with_metadata_observer(
    paths: &ArchiveDeletePaths,
    thread_id: &str,
    mut after_metadata_check: impl FnMut(&Path),
) -> Result<ArchiveDeleteResult, ArchiveDeleteError> {
    let rollout = load_archived_rollout(paths, thread_id)?;
    ensure_rollout_is_contained(&rollout, &paths.archived_sessions)?;
    scrub::validate_json_state(&paths.bridge_state)?;
    scrub::validate_json_state(&paths.global_state)?;
    let backup_dir = create_backup_dir(paths, thread_id)?;
    let backup_paths = backup_inputs(paths, &backup_dir, &rollout)?;
    let mut completed = "backup";
    let result = (|| {
        for metadata in [
            &paths.bridge_state,
            &paths.global_state,
            &paths.session_index,
        ] {
            verification::unchanged_optional_file(metadata, &backup_dir)?;
        }
        scrub::validate_json_state(&paths.bridge_state)?;
        scrub::validate_json_state(&paths.global_state)?;
        let transcript = transcript::Transcript::verified(
            &rollout,
            &backup_dir.join("transcript/rollout.jsonl"),
        )?;
        delete_state_row(paths, thread_id, &rollout, &backup_dir)?;
        completed = "state row deletion";
        let deleted_log_rows = delete_log_rows(paths, thread_id, &backup_dir)?;
        completed = "log deletion";
        verification::unchanged_optional_file(&paths.bridge_state, &backup_dir)?;
        after_metadata_check(&paths.bridge_state);
        let bridge_state_scrubbed =
            scrub_json_state(&paths.bridge_state, thread_id, true, &backup_dir)?;
        completed = "bridge state update";
        verification::unchanged_optional_file(&paths.global_state, &backup_dir)?;
        after_metadata_check(&paths.global_state);
        let global_state_scrubbed =
            scrub_json_state(&paths.global_state, thread_id, false, &backup_dir)?;
        completed = "global state update";
        verification::unchanged_optional_file(&paths.session_index, &backup_dir)?;
        after_metadata_check(&paths.session_index);
        let session_index_removed =
            scrub_session_index(&paths.session_index, thread_id, &backup_dir)?;
        completed = "session index update";
        ensure_rollout_is_contained(&rollout, &paths.archived_sessions)?;
        transcript.delete()?;
        completed = "rollout deletion";
        verify_deleted(paths, thread_id, &rollout)?;
        Ok(ArchiveDeleteResult {
            backup_dir: backup_dir.clone(),
            backup_paths,
            deleted_log_rows,
            deleted_rollout_path: rollout,
            bridge_state_scrubbed,
            global_state_scrubbed,
            session_index_removed,
        })
    })();
    result.map_err(|source| ArchiveDeleteError::Partial {
        last_completed_stage: completed,
        backup_dir,
        source: Box::new(source),
    })
}

fn load_archived_rollout(
    paths: &ArchiveDeletePaths,
    thread_id: &str,
) -> Result<PathBuf, ArchiveDeleteError> {
    let connection =
        Connection::open_with_flags(&paths.state_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let record = connection
        .query_row(
            "SELECT rollout_path, archived FROM threads WHERE id = ?1",
            [thread_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((rollout, archived)) = record else {
        return Err(ArchiveDeleteError::MissingThread(thread_id.to_owned()));
    };
    if archived != 1 {
        return Err(ArchiveDeleteError::ActiveThread);
    }
    Ok(PathBuf::from(rollout))
}

fn ensure_rollout_is_contained(rollout: &Path, root: &Path) -> Result<(), ArchiveDeleteError> {
    let root = fs::canonicalize(root).map_err(|source| io_error(root, source))?;
    let candidate = if rollout.exists() {
        fs::canonicalize(rollout).map_err(|source| io_error(rollout, source))?
    } else {
        let parent = rollout.parent().ok_or(ArchiveDeleteError::OutsideArchive)?;
        fs::canonicalize(parent)
            .map_err(|source| io_error(parent, source))?
            .join(
                rollout
                    .file_name()
                    .ok_or(ArchiveDeleteError::OutsideArchive)?,
            )
    };
    if candidate == root || !candidate.starts_with(root) {
        return Err(ArchiveDeleteError::OutsideArchive);
    }
    Ok(())
}

fn io_error(path: &Path, source: std::io::Error) -> ArchiveDeleteError {
    ArchiveDeleteError::Io {
        path: path.to_owned(),
        source,
    }
}
