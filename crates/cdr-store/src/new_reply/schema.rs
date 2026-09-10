use crate::Result;
use rusqlite::Connection;

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS codex_new_first_replies (
            job_id TEXT PRIMARY KEY, ingress_id TEXT NOT NULL UNIQUE,
            identity_json TEXT NOT NULL, turn_id TEXT, accepted_at REAL,
            state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','verified','review_required')),
            version INTEGER NOT NULL DEFAULT 1, scan_json TEXT NOT NULL DEFAULT '{}',
            last_error TEXT NOT NULL DEFAULT '', confirmation_delivered INTEGER NOT NULL DEFAULT 0,
            warning_due INTEGER NOT NULL DEFAULT 0, checked_at REAL NOT NULL DEFAULT 0,
            ack_recovery_allowed INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX IF NOT EXISTS codex_new_first_replies_pending
            ON codex_new_first_replies(checked_at,job_id);"
    )?;
    Ok(())
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='codex_new_first_replies')",
        [], |row| row.get(0),
    )?)
}
