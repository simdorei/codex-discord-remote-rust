use crate::Result;
use rusqlite::Connection;

pub(crate) fn migrate_cancellation_schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS codex_request_cancellations (
        job_id TEXT PRIMARY KEY, target_thread_id TEXT NOT NULL, channel_id INTEGER NOT NULL,
        owner_user_id INTEGER NOT NULL, discord_message_id INTEGER, cancelled_at REAL NOT NULL);
        CREATE UNIQUE INDEX IF NOT EXISTS codex_cancelled_message ON codex_request_cancellations(discord_message_id)
        WHERE discord_message_id IS NOT NULL;")?;
    for table in ["codex_turn_queue", "codex_prompt_intakes"] {
        for operation in ["INSERT", "UPDATE"] {
            db.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS cancelled_{table}_{operation}
                 BEFORE {operation} ON {table}
                 WHEN EXISTS(SELECT 1 FROM codex_request_cancellations c WHERE c.job_id=NEW.job_id
                    OR (NEW.discord_message_id IS NOT NULL AND c.discord_message_id=NEW.discord_message_id))
                 BEGIN SELECT RAISE(ABORT, 'request was cancelled by its original sender; no automatic retry'); END;"
            ))?;
        }
    }
    Ok(())
}

pub(crate) fn cancellation_schema_current(db: &Connection) -> Result<bool> {
    let count: i64 = db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE
        (type='table' AND name='codex_request_cancellations') OR
        (type='index' AND name='codex_cancelled_message') OR
        (type='trigger' AND name IN ('cancelled_codex_turn_queue_INSERT','cancelled_codex_turn_queue_UPDATE',
        'cancelled_codex_prompt_intakes_INSERT','cancelled_codex_prompt_intakes_UPDATE'))", [], |row| row.get(0))?;
    Ok(count == 6)
}
