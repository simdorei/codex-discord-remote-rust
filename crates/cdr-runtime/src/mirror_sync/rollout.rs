//! Manual sync restores missing plain logs and safely repairs mid-record read positions.
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
            let mut connection = cdr_store::schema::open_initialized(db)?;
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(cdr_store::StoreError::from)?;
            let recovery = validate_cursor(&tx, thread, &mut file)?;
            tx.commit().map_err(cdr_store::StoreError::from)?;
            report_recovery(thread, recovery);
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
    // any file that appeared meanwhile. A cursor inside a verified complete record
    // may be rewound to that record's start; context and event rows are preserved.
    let mut connection = cdr_store::schema::open_initialized(db)?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(cdr_store::StoreError::from)?;
    let recovery = validate_cursor(&tx, thread, staged.file.as_file_mut())?;
    staged.publish(path).map_err(|error| io_error(&error))?;
    tx.commit().map_err(cdr_store::StoreError::from)?;
    report_recovery(thread, recovery);
    eprintln!(
        "mirror_sync_rollout_restored thread={} cursor={}",
        thread.id,
        if recovery.is_some() {
            "recovered"
        } else {
            "preserved"
        }
    );
    Ok(true)
}

fn validate_cursor(
    connection: &rusqlite::Connection,
    thread: &ThreadInfo,
    file: &mut File,
) -> Result<Option<cursor::Recovery>, MirrorSyncError> {
    let offset = cursor::read(connection, &thread.id).map_err(cdr_store::StoreError::from)?;
    let recovery = cursor::validate(file, &thread.rollout_path, offset.as_ref())
        .map_err(|error| io_error(&error))?;
    if let Some(recovery) = recovery
        && !cursor::persist(
            connection,
            &thread.id,
            &thread.rollout_path,
            recovery,
            super::now()?,
        )
        .map_err(cdr_store::StoreError::from)?
    {
        return Err(MirrorSyncError::Invalid(format!(
            "thread={} rollout cursor changed during recovery; retry sync",
            thread.id
        )));
    }
    Ok(recovery)
}

fn report_recovery(thread: &ThreadInfo, recovery: Option<cursor::Recovery>) {
    if let Some(recovery) = recovery {
        eprintln!(
            "mirror_sync_cursor_recovered thread={} from={} to={} policy=rewind_containing_record",
            thread.id, recovery.from, recovery.to
        );
    }
}

fn io_error(error: &io::Error) -> MirrorSyncError {
    MirrorSyncError::Invalid(error.to_string())
}
