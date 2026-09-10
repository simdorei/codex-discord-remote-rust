use rusqlite::Connection;

use crate::Result;

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS codex_app_server_runtime (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1), runtime_id TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS codex_dead_generation_incidents (
            runtime_id TEXT NOT NULL, generation INTEGER NOT NULL,
            snapshot_json TEXT NOT NULL, queue_jobs_json TEXT NOT NULL,
            created_at REAL NOT NULL, PRIMARY KEY(runtime_id, generation));
         CREATE TABLE IF NOT EXISTS codex_dead_generation_holds (
            target_thread_id TEXT PRIMARY KEY, runtime_id TEXT NOT NULL,
            generation INTEGER NOT NULL, created_at REAL NOT NULL);",
    )?;
    Ok(())
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT COUNT(*) = 3 FROM sqlite_schema WHERE type = 'table' AND name IN \
         ('codex_app_server_runtime', 'codex_dead_generation_incidents', 'codex_dead_generation_holds')",
        [],
        |row| row.get(0),
    )?)
}
