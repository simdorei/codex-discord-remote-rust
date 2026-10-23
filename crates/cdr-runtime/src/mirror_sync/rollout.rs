//! Manual sync restores missing plain logs without changing saved read positions.
use super::MirrorSyncError;
use cdr_codex_state::ThreadInfo;
use rusqlite::TransactionBehavior;
use std::{fs::File, io, path::Path};

mod compressed;
mod cursor;

pub(super) fn ensure(db: &Path, thread: &ThreadInfo) -> Result<bool, MirrorSyncError> {
    ensure_inner(db, thread)
        .map_err(|error| MirrorSyncError::Invalid(format!("thread={} rollout: {error}", thread.id)))
}

fn ensure_inner(db: &Path, thread: &ThreadInfo) -> Result<bool, MirrorSyncError> {
    let path = &thread.rollout_path;
    match File::open(path) {
        Ok(mut file) => {
            let offset = cdr_store::mirror::get_offset(db, &thread.id)?;
            cursor::validate(&mut file, path, offset.as_ref()).map_err(|error| io_error(&error))?;
            return Ok(true);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(&error)),
    }
    let Some(mut staged) = compressed::stage(path, &thread.id).map_err(|error| io_error(&error))?
    else {
        return Ok(false);
    };
    // Decode outside the store lock. Read the latest cursor under the same writer
    // lock used by the mirror worker, then publish a complete file without replacing
    // any file that appeared meanwhile. No cursor/context/event row is changed.
    let mut connection = cdr_store::schema::open_initialized(db)?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(cdr_store::StoreError::from)?;
    let offset = cursor::read(&tx, &thread.id).map_err(cdr_store::StoreError::from)?;
    cursor::validate(staged.file.as_file_mut(), path, offset.as_ref())
        .map_err(|error| io_error(&error))?;
    staged.publish(path).map_err(|error| io_error(&error))?;
    tx.commit().map_err(cdr_store::StoreError::from)?;
    eprintln!(
        "mirror_sync_rollout_restored thread={} cursor=preserved",
        thread.id
    );
    Ok(true)
}

fn io_error(error: &io::Error) -> MirrorSyncError {
    MirrorSyncError::Invalid(error.to_string())
}
