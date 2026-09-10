use crate::{Result, queue::StoredQueueJob, schema::open_initialized};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::path::Path;

#[must_use]
pub fn user_origin_marker(thread: &str, turn: &str, prompt: &str) -> String {
    let digest = hex::encode(Sha256::digest(prompt.trim().as_bytes()));
    format!("discord-user:v1:{thread}:{turn}:{digest}")
}

pub fn record_user_origin(
    path: &Path,
    thread: &str,
    turn: &str,
    prompt: &str,
    now: f64,
) -> Result<()> {
    record(&open_initialized(path)?, thread, turn, prompt, now)
}

pub(crate) fn record_job_origin(connection: &Connection, job: &StoredQueueJob) -> Result<()> {
    if let Some(turn) = job.turn_id.as_deref() {
        record(
            connection,
            &job.target_thread_id,
            turn,
            &job.prompt,
            job.updated_at,
        )?;
    }
    Ok(())
}

fn record(connection: &Connection, thread: &str, turn: &str, prompt: &str, now: f64) -> Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO codex_session_mirror_events (event_digest, codex_thread_id, created_at) VALUES (?, ?, ?)",
        params![user_origin_marker(thread, turn, prompt), thread, now],
    )?;
    Ok(())
}
