//! Complete the recovery copy before any destructive archive operation.
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, MAIN_DB, OpenFlags};
use uuid::Uuid;

use super::{ArchiveDeleteError, ArchiveDeletePaths, io_error};

pub(super) fn create_backup_dir(
    paths: &ArchiveDeletePaths,
    thread_id: &str,
) -> Result<PathBuf, ArchiveDeleteError> {
    fs::create_dir_all(&paths.backup_root)
        .map_err(|source| io_error(&paths.backup_root, source))?;
    let prefix = thread_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect::<String>();
    let path = paths.backup_root.join(format!(
        "delete-archive-{}-{prefix}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir(&path).map_err(|source| io_error(&path, source))?;
    Ok(path)
}

pub(super) fn backup_inputs(
    paths: &ArchiveDeletePaths,
    backup_dir: &Path,
    rollout: &Path,
) -> Result<Vec<PathBuf>, ArchiveDeleteError> {
    let mut names = std::collections::BTreeSet::new();
    for source in [
        &paths.state_db,
        &paths.log_db,
        &paths.global_state,
        &paths.bridge_state,
        &paths.session_index,
    ] {
        let name = file_name(source)?.to_string_lossy().to_lowercase();
        if name == "transcript" || !names.insert(name) {
            return Err(io_error(
                source,
                std::io::Error::other("archive backup filename collision"),
            ));
        }
    }
    let mut copied = vec![backup_sqlite(&paths.state_db, backup_dir)?];
    if paths.log_db.exists() {
        copied.push(backup_sqlite(&paths.log_db, backup_dir)?);
    }
    for source in [
        &paths.global_state,
        &paths.bridge_state,
        &paths.session_index,
    ] {
        if source.exists() {
            let destination = backup_dir.join(file_name(source)?);
            fs::copy(source, &destination).map_err(|error| io_error(source, error))?;
            copied.push(destination);
        }
    }
    // Use a dedicated directory so a configurable metadata filename cannot collide.
    let transcript_dir = backup_dir.join("transcript");
    fs::create_dir(&transcript_dir).map_err(|error| io_error(&transcript_dir, error))?;
    let destination = transcript_dir.join("rollout.jsonl");
    fs::copy(rollout, &destination).map_err(|error| io_error(rollout, error))?;
    super::verification::unchanged_file(rollout, &destination)?;
    copied.push(destination);
    Ok(copied)
}

fn backup_sqlite(source: &Path, backup_dir: &Path) -> Result<PathBuf, ArchiveDeleteError> {
    let destination = backup_dir.join(file_name(source)?);
    let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.backup(MAIN_DB, &destination, None)?;
    Ok(destination)
}

fn file_name(path: &Path) -> Result<&std::ffi::OsStr, ArchiveDeleteError> {
    path.file_name()
        .ok_or_else(|| io_error(path, std::io::Error::other("missing file name")))
}
