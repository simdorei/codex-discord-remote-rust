use super::{ArchiveDeleteError, io_error};
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub(super) fn file_digest(path: &Path) -> Result<[u8; 32], ArchiveDeleteError> {
    let mut file = File::open(path).map_err(|e| io_error(path, e))?;
    reader_digest(&mut file).map_err(|e| io_error(path, e))
}

pub(super) fn reader_digest(file: &mut impl Read) -> std::io::Result<[u8; 32]> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

pub(super) fn unchanged_file(source: &Path, backup: &Path) -> Result<(), ArchiveDeleteError> {
    if file_digest(source)? != file_digest(backup)? {
        return Err(ArchiveDeleteError::ConcurrentMutation);
    }
    Ok(())
}

pub(super) fn unchanged_optional_file(
    source: &Path,
    backup_dir: &Path,
) -> Result<(), ArchiveDeleteError> {
    let backup = backup_dir.join(
        source
            .file_name()
            .ok_or(ArchiveDeleteError::ConcurrentMutation)?,
    );
    match (
        source.try_exists().map_err(|e| io_error(source, e))?,
        backup.try_exists().map_err(|e| io_error(&backup, e))?,
    ) {
        (false, false) => Ok(()),
        (true, true) => unchanged_file(source, &backup),
        _ => Err(ArchiveDeleteError::ConcurrentMutation),
    }
}

pub(super) fn unchanged_rows(
    connection: &Connection,
    backup_db: &Path,
    sql: &str,
    id: &str,
) -> Result<(), ArchiveDeleteError> {
    let backup = Connection::open_with_flags(backup_db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if row_digests(connection, sql, id)? != row_digests(&backup, sql, id)? {
        return Err(ArchiveDeleteError::ConcurrentMutation);
    }
    Ok(())
}

fn row_digests(
    connection: &Connection,
    sql: &str,
    id: &str,
) -> Result<Vec<[u8; 32]>, ArchiveDeleteError> {
    let mut statement = connection.prepare(sql)?;
    let count = statement.column_count();
    let names = statement.column_names().join("\0");
    let mut rows = statement.query([id])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let mut digest = Sha256::new();
        digest.update(names.as_bytes());
        for column in 0..count {
            match row.get_ref(column)? {
                ValueRef::Null => digest.update([0]),
                ValueRef::Integer(value) => {
                    digest.update([1]);
                    digest.update(value.to_le_bytes());
                }
                ValueRef::Real(value) => {
                    digest.update([2]);
                    digest.update(value.to_bits().to_le_bytes());
                }
                ValueRef::Text(value) => {
                    digest.update([3]);
                    digest.update(value.len().to_le_bytes());
                    digest.update(value);
                }
                ValueRef::Blob(value) => {
                    digest.update([4]);
                    digest.update(value.len().to_le_bytes());
                    digest.update(value);
                }
            }
        }
        result.push(digest.finalize().into());
    }
    result.sort_unstable();
    Ok(result)
}
