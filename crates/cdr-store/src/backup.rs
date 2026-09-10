use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{Connection, MAIN_DB, OpenFlags};
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
    if let Err(error) = source.backup(MAIN_DB, &target, None) {
        remove_incomplete(&target)?;
        return Err(error.into());
    }
    if let Err(error) = validate_snapshot(&target, version) {
        remove_incomplete(&target)?;
        return Err(error);
    }
    Ok(target)
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
