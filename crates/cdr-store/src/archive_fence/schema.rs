use crate::Result;
use rusqlite::Connection;

// Additive v1 Rust extension of the shared v2 database. SQLite triggers also
// enforce the fence for older callers; initialization backs up before adding it.
pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(include_str!("schema.sql"))?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT COUNT(*)=9 FROM sqlite_schema WHERE
         (type='table' AND name='codex_archive_fences') OR
         (type='view' AND name='cdr_archive_inspections_v1') OR
         (type='trigger' AND name IN ('cdr_archive_admission_v1','cdr_archive_execution_v1',
          'cdr_archive_held_v1','cdr_archive_queue_insert_v1','cdr_archive_queue_update_v1',
          'cdr_archive_intake_insert_v1','cdr_archive_intake_update_v1'))",
        [],
        |row| row.get(0),
    )?)
}
