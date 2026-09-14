use crate::Result;
use rusqlite::Connection;

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS cdr_async_question_inbox (
        id TEXT PRIMARY KEY, runtime_id TEXT NOT NULL, generation INTEGER NOT NULL,
        thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, item_id TEXT NOT NULL,
        candidate_job_id TEXT NOT NULL, candidate_channel_id INTEGER NOT NULL,
        candidate_owner_id INTEGER NOT NULL, body TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'waiting', created_at REAL NOT NULL);
        CREATE INDEX IF NOT EXISTS cdr_async_question_inbox_pending ON cdr_async_question_inbox(runtime_id,generation,state);")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS cdr_async_questions (
        id TEXT PRIMARY KEY, runtime_id TEXT NOT NULL, generation INTEGER NOT NULL,
        thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, item_id TEXT NOT NULL,
        origin_job_id TEXT NOT NULL, channel_id INTEGER NOT NULL, owner_user_id INTEGER NOT NULL,
        body TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'observed', message_id TEXT, chosen INTEGER,
        dispatch_mode TEXT, reply_job_id TEXT, accepted_turn_id TEXT, error TEXT NOT NULL DEFAULT '',
        owner_confirmed INTEGER NOT NULL DEFAULT 0,
        created_at REAL NOT NULL, updated_at REAL NOT NULL);
        CREATE INDEX IF NOT EXISTS cdr_async_question_pending ON cdr_async_questions(runtime_id,state);
        CREATE UNIQUE INDEX IF NOT EXISTS cdr_async_question_reply_job ON cdr_async_questions(reply_job_id) WHERE reply_job_id IS NOT NULL;")?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row("SELECT COUNT(*)=2 FROM sqlite_schema WHERE type='table' AND name IN ('cdr_async_questions','cdr_async_question_inbox')", [], |r|r.get(0))?)
}
