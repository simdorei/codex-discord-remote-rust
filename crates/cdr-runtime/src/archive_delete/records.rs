//! Transactional deletion and verification of archived `SQLite` records.
use super::{ArchiveDeleteError, ArchiveDeletePaths, verification};
use rusqlite::{Connection, TransactionBehavior};
use std::path::Path;

pub(super) fn delete_state_row(
    paths: &ArchiveDeletePaths,
    thread_id: &str,
    rollout: &Path,
    backup_dir: &Path,
) -> Result<(), ArchiveDeleteError> {
    let mut connection = Connection::open(&paths.state_db)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND archived=1 AND rollout_path=?2)",
        (thread_id, rollout.to_string_lossy().as_ref()),
        |r| r.get(0),
    )?;
    if !current {
        return Err(ArchiveDeleteError::ConcurrentMutation);
    }
    let backup_db = backup_dir.join(
        paths
            .state_db
            .file_name()
            .ok_or(ArchiveDeleteError::ConcurrentMutation)?,
    );
    verification::unchanged_rows(
        &transaction,
        &backup_db,
        "SELECT * FROM threads WHERE id=?1",
        thread_id,
    )?;
    verification::unchanged_rows(
        &transaction,
        &backup_db,
        "SELECT * FROM thread_spawn_edges WHERE child_thread_id=?1 OR parent_thread_id=?1",
        thread_id,
    )?;
    transaction.execute(
        "DELETE FROM thread_spawn_edges WHERE child_thread_id = ?1 OR parent_thread_id = ?1",
        [thread_id],
    )?;
    let deleted = transaction.execute(
        "DELETE FROM threads WHERE id = ?1 AND archived = 1 AND rollout_path = ?2",
        (thread_id, rollout.to_string_lossy().as_ref()),
    )?;
    if deleted != 1 {
        return Err(ArchiveDeleteError::ConcurrentMutation);
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn delete_log_rows(
    paths: &ArchiveDeletePaths,
    thread_id: &str,
    backup_dir: &Path,
) -> Result<usize, ArchiveDeleteError> {
    if !paths.log_db.exists() {
        return Ok(0);
    }
    let mut connection = Connection::open(&paths.log_db)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let backup_db = backup_dir.join(
        paths
            .log_db
            .file_name()
            .ok_or(ArchiveDeleteError::ConcurrentMutation)?,
    );
    verification::unchanged_rows(
        &transaction,
        &backup_db,
        "SELECT * FROM logs WHERE thread_id=?1",
        thread_id,
    )?;
    let deleted = transaction.execute("DELETE FROM logs WHERE thread_id = ?1", [thread_id])?;
    transaction.commit()?;
    Ok(deleted)
}

pub(super) fn verify_deleted(
    paths: &ArchiveDeletePaths,
    thread_id: &str,
    rollout: &Path,
) -> Result<(), ArchiveDeleteError> {
    let state = Connection::open(&paths.state_db)?;
    let remaining: i64 = state.query_row(
        "SELECT COUNT(*) FROM threads WHERE id = ?1",
        [thread_id],
        |row| row.get(0),
    )?;
    if remaining != 0 {
        return Err(ArchiveDeleteError::ThreadStillPresent);
    }
    if paths.log_db.exists() {
        let logs = Connection::open(&paths.log_db)?;
        let remaining: i64 = logs.query_row(
            "SELECT COUNT(*) FROM logs WHERE thread_id = ?1",
            [thread_id],
            |row| row.get(0),
        )?;
        if remaining != 0 {
            return Err(ArchiveDeleteError::LogsStillPresent);
        }
    }
    if rollout.exists() {
        return Err(ArchiveDeleteError::RolloutStillPresent);
    }
    Ok(())
}
