use crate::Result;
use rusqlite::Connection;

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cdr_server_responses(
        request_key TEXT PRIMARY KEY, runtime_id TEXT NOT NULL, resident_owner TEXT NOT NULL,
        generation INTEGER NOT NULL CHECK(generation>0), target_thread_id TEXT NOT NULL,
        turn_id TEXT NOT NULL, job_id TEXT NOT NULL, authority_json TEXT NOT NULL,
        response_sha256 TEXT NOT NULL,
        phase TEXT NOT NULL CHECK(phase IN ('admitted','flushed','not_sent','terminal')),
        created_at REAL NOT NULL, updated_at REAL NOT NULL, terminal_json TEXT);
        CREATE INDEX IF NOT EXISTS cdr_server_response_target
        ON cdr_server_responses(target_thread_id,phase);",
    )?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT (SELECT count(*) FROM pragma_table_info('cdr_server_responses'))=13
        AND EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cdr_server_response_target')",
        [],
        |row| row.get(0),
    )?)
}
