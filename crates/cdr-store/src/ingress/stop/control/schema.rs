use crate::Result;
use rusqlite::Connection;

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    super::super::revision::migrate_schema(db)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS cdr_stop_controls(
        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
        operation_id TEXT NOT NULL UNIQUE,
        target_thread_id TEXT NOT NULL,
        resident_owner TEXT NOT NULL,
        generation INTEGER NOT NULL CHECK(generation>0),
        turn_id TEXT NOT NULL,
        record_json TEXT NOT NULL,
        phase TEXT NOT NULL CHECK(phase IN ('accepted','dispatching','acknowledged','unknown','settled')),
        claim_token TEXT, wire_attempt TEXT, wire_id TEXT,
        last_error TEXT NOT NULL DEFAULT '', terminal_json TEXT);
        CREATE INDEX IF NOT EXISTS cdr_stop_pending ON cdr_stop_controls(phase,sequence);
        CREATE INDEX IF NOT EXISTS cdr_stop_target ON cdr_stop_controls(target_thread_id,phase);
        CREATE UNIQUE INDEX IF NOT EXISTS cdr_stop_original_interrupt
        ON cdr_stop_controls(target_thread_id,resident_owner,generation,turn_id)
        WHERE claim_token IS NOT NULL;")?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    if !super::super::revision::schema_current(db)? {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT
        (SELECT COUNT(*) FROM pragma_table_info('cdr_stop_controls'))=13
        AND EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cdr_stop_original_interrupt')
        AND EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cdr_stop_pending')
        AND EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cdr_stop_target')",
        [],
        |row| row.get(0),
    )?)
}
