use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, ErrorCode, OpenFlags};
use uuid::Uuid;

use crate::schema::{STORE_BUSY_TIMEOUT, assert_integrity, schema_version};
use crate::{Result, StoreError};

const BACKUP_DIRECTORY: &str = ".codex-discord-backups";

pub fn snapshot(path: &Path) -> Result<PathBuf> {
    if !path.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("store database was not found: {}", path.display()),
        )
        .into());
    }
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(STORE_BUSY_TIMEOUT)?;
    assert_integrity(&source)?;
    let version = schema_version(&source)?;
    let target = snapshot_path(path, version)?;
    if let Err(error) = copy_snapshot(&source, &target, Duration::from_secs(30)) {
        remove_incomplete(&target)?;
        return Err(error.into());
    }
    if let Err(error) = validate_snapshot(&target, version) {
        remove_incomplete(&target)?;
        return Err(error);
    }
    Ok(target)
}

fn copy_snapshot(source: &Connection, target: &Path, budget: Duration) -> rusqlite::Result<()> {
    let began = Instant::now();
    let read = source.unchecked_transaction()?;
    loop {
        require_backup_budget(began, budget)?;
        match read.query_row("SELECT count(*) FROM sqlite_schema", [], |r| {
            r.get::<_, i64>(0)
        }) {
            Ok(_) => break,
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
                ) =>
            {
                wait_for_writer(began, budget);
            }
            Err(error) => return Err(error),
        }
    }
    // Keep one read snapshot while copying. Without it, frequent commits in
    // rollback-journal mode can repeatedly restart SQLite's incremental copy.
    let mut destination = Connection::open(target)?;
    let backup = Backup::new(&read, &mut destination)?;
    loop {
        require_backup_budget(began, budget)?;
        match backup.step(128)? {
            StepResult::Done => return Ok(()),
            StepResult::More => {}
            StepResult::Busy | StepResult::Locked => {
                // Connection::backup returns immediately on these recoverable
                // states. A live bridge can briefly write between backup steps.
                wait_for_writer(began, budget);
            }
            _ => return Err(rusqlite::Error::InvalidQuery),
        }
    }
}

fn require_backup_budget(began: Instant, budget: Duration) -> rusqlite::Result<()> {
    if began.elapsed() >= budget {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("online backup exceeded its bounded wait; incomplete snapshot discarded".into()),
        ));
    }
    Ok(())
}

fn wait_for_writer(began: Instant, budget: Duration) {
    std::thread::sleep(Duration::from_millis(50).min(budget.saturating_sub(began.elapsed())));
}

fn snapshot_path(path: &Path, version: i64) -> Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let directory = parent.join(BACKUP_DIRECTORY);
    fs::create_dir_all(&directory)?;
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("store");
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let unique = &Uuid::new_v4().simple().to_string()[..12];
    Ok(directory.join(format!(
        "{stem}.v{version}-cutover.{timestamp}.{unique}.sqlite"
    )))
}

fn validate_snapshot(path: &Path, expected_version: i64) -> Result<()> {
    let snapshot = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    assert_integrity(&snapshot)?;
    let actual_version = schema_version(&snapshot)?;
    if actual_version != expected_version {
        return Err(StoreError::UnsupportedVersion {
            found: actual_version,
            supported: expected_version,
        });
    }
    Ok(())
}

fn remove_incomplete(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;
