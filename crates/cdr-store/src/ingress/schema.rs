use rusqlite::Connection;

use crate::Result;

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS discord_ingress_journal (
            ingress_id TEXT PRIMARY KEY,
            version INTEGER NOT NULL DEFAULT 1 CHECK(version = 1),
            kind TEXT NOT NULL CHECK(kind IN ('message', 'interaction', 'action')),
            event_id INTEGER,
            application_id INTEGER,
            channel_id INTEGER NOT NULL,
            owner_user_id INTEGER NOT NULL,
            source_message_id INTEGER,
            payload_json TEXT NOT NULL,
            runtime_id TEXT,
            state TEXT NOT NULL CHECK(state IN ('staged','acknowledged','executing','owned','completed','held')),
            phase TEXT NOT NULL,
            target_thread_id TEXT,
            canonical_owner TEXT,
            owner_kind TEXT,
            owner_id TEXT,
            outcome_json TEXT,
            confirmation_delivered INTEGER NOT NULL DEFAULT 0,
            hold_reason TEXT NOT NULL DEFAULT '',
            notice_staged INTEGER NOT NULL DEFAULT 0,
            created_at REAL NOT NULL,
            updated_at REAL NOT NULL
         );
         CREATE UNIQUE INDEX IF NOT EXISTS discord_ingress_event ON discord_ingress_journal(kind,event_id) WHERE event_id IS NOT NULL;
         CREATE INDEX IF NOT EXISTS discord_ingress_owner ON discord_ingress_journal(canonical_owner);
         CREATE TABLE IF NOT EXISTS discord_ingress_owner_receipts (
            owner_key TEXT PRIMARY KEY,
            owner_kind TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            target_thread_id TEXT,
            channel_id INTEGER NOT NULL,
            owner_user_id INTEGER NOT NULL,
            payload_json TEXT NOT NULL,
            created_at REAL NOT NULL
         );",
    )?;
    Ok(())
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT COUNT(*) = 2 FROM sqlite_schema WHERE type = 'table'
         AND name IN ('discord_ingress_journal','discord_ingress_owner_receipts')",
        [],
        |row| row.get(0),
    )?)
}
