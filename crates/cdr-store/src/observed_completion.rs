//! Terminal observations are durable before history lookup or Discord I/O.
use crate::{Result, schema::open_initialized};
use rusqlite::{Connection, params};
use std::path::Path;

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS codex_observed_completions (
        thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, generation INTEGER NOT NULL,
        payload TEXT NOT NULL, last_error TEXT NOT NULL DEFAULT '',
        PRIMARY KEY(thread_id, turn_id));",
    )?;
    Ok(())
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table'
        AND name='codex_observed_completions')",
        [],
        |row| row.get(0),
    )?)
}

/// Never claim an app-owned turn or a terminal event from an old generation.
pub fn record(
    path: &Path,
    thread: &str,
    turn: &str,
    generation: i64,
    payload: &str,
) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "INSERT OR IGNORE INTO codex_observed_completions
        (thread_id,turn_id,generation,payload)
        SELECT ?,?,?,? WHERE EXISTS(SELECT 1 FROM codex_turn_queue
        WHERE target_thread_id=? AND turn_id=? AND app_server_generation=? AND state='running')",
        params![thread, turn, generation, payload, thread, turn, generation],
    )? == 1)
}

pub fn contains(path: &Path, thread: &str, turn: &str) -> Result<bool> {
    Ok(open_initialized(path)?.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_observed_completions
        WHERE thread_id=? AND turn_id=?)",
        params![thread, turn],
        |row| row.get(0),
    )?)
}

pub fn pending(path: &Path) -> Result<Vec<(String, String, String)>> {
    Ok(pending_with_generation(path)?
        .into_iter()
        .map(|(thread, turn, _, payload)| (thread, turn, payload))
        .collect())
}

pub fn pending_with_generation(path: &Path) -> Result<Vec<(String, String, i64, String)>> {
    let connection = open_initialized(path)?;
    let mut query = connection.prepare(
        "SELECT thread_id,turn_id,generation,payload FROM codex_observed_completions ORDER BY rowid",
    )?;
    Ok(query
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn finish(path: &Path, thread: &str, turn: &str) -> Result<()> {
    open_initialized(path)?.execute(
        "DELETE FROM codex_observed_completions WHERE thread_id=? AND turn_id=?",
        params![thread, turn],
    )?;
    Ok(())
}

pub fn record_error(path: &Path, thread: &str, turn: &str, error: &str) -> Result<()> {
    let bounded: String = error.chars().take(1000).collect();
    open_initialized(path)?.execute(
        "UPDATE codex_observed_completions SET last_error=? WHERE thread_id=? AND turn_id=?",
        params![bounded, thread, turn],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{NewQueueJob, begin_attempt, enqueue, mark_running};

    #[test]
    fn only_exact_owned_generation_is_recorded_and_record_survives_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store.sqlite");
        enqueue(
            &path,
            NewQueueJob {
                job_id: "job",
                target_thread_id: "thread",
                channel_id: 1,
                owner_user_id: Some(2),
                discord_message_id: None,
                app_server_generation: 7,
                prompt: "input",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        begin_attempt(&path, "job", &[], 7).unwrap();
        mark_running(&path, "job", "turn", 7).unwrap();
        assert!(!record(&path, "other", "turn", 7, "{}").unwrap());
        assert!(!record(&path, "thread", "turn", 6, "{}").unwrap());
        assert!(record(&path, "thread", "turn", 7, "terminal").unwrap());
        assert!(!record(&path, "thread", "turn", 7, "changed").unwrap());
        assert!(contains(&path, "thread", "turn").unwrap());
        assert_eq!(
            pending(&path).unwrap(),
            vec![("thread".into(), "turn".into(), "terminal".into())]
        );
        record_error(&path, "thread", "turn", "read failed").unwrap();
        assert!(contains(&path, "thread", "turn").unwrap());
        finish(&path, "thread", "turn").unwrap();
        assert!(pending(&path).unwrap().is_empty());
    }
}
