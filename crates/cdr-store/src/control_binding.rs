//! A busy button can control only its original turn or one preceding job.
use crate::{Result, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS codex_busy_control_bindings (
        choice_id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, turn_id TEXT, job_id TEXT);
        CREATE TRIGGER IF NOT EXISTS codex_bind_preparing_control
        AFTER UPDATE OF turn_id ON codex_turn_queue
        WHEN OLD.turn_id IS NULL AND NEW.turn_id IS NOT NULL
        BEGIN
          UPDATE codex_busy_control_bindings SET turn_id=NEW.turn_id
          WHERE job_id=NEW.job_id AND thread_id=NEW.target_thread_id AND turn_id IS NULL;
        END;",
    )?;
    Ok(())
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table'
        AND name='codex_busy_control_bindings') AND EXISTS(SELECT 1 FROM sqlite_schema
        WHERE type='trigger' AND name='codex_bind_preparing_control')",
        [],
        |row| row.get(0),
    )?)
}

pub fn bind(
    path: &Path,
    choice: &str,
    thread: &str,
    turn: Option<&str>,
    job: Option<&str>,
) -> Result<()> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute("DELETE FROM codex_busy_control_bindings WHERE choice_id NOT IN (SELECT choice_id FROM busy_choices)",[])?;
    transaction.execute(
        "INSERT OR IGNORE INTO codex_busy_control_bindings (choice_id,thread_id,turn_id,job_id)
        VALUES (?,?,?,?)",
        params![choice, thread, turn, job],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Resolve once, only from the exact preceding job and only while it is running.
/// An absent/ambiguous binding must request a fresh choice, never the next turn.
pub fn resolve(path: &Path, choice: &str, thread: &str) -> Result<Option<String>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(Option<String>,Option<String>)> = transaction.query_row(
        "SELECT turn_id,job_id FROM codex_busy_control_bindings WHERE choice_id=? AND thread_id=?",
        params![choice,thread],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let result = match row {
        Some((Some(turn), _)) => Some(turn),
        Some((None, Some(job))) => {
            let turn: Option<String> = transaction
                .query_row(
                    "SELECT turn_id FROM codex_turn_queue
                WHERE job_id=? AND target_thread_id=? AND state='running' AND goal_waiting=0",
                    params![job, thread],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            if let Some(turn) = &turn {
                transaction.execute("UPDATE codex_busy_control_bindings SET turn_id=? WHERE choice_id=? AND turn_id IS NULL",
                    params![turn,choice])?;
            }
            turn
        }
        _ => None,
    };
    transaction.commit()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{NewBusyChoice, create_busy_choice};
    use crate::queue::{
        NewQueueJob, attach_goal_turn, begin_attempt, enqueue, mark_goal_waiting, mark_running,
    };

    #[test]
    fn preparing_button_keeps_first_turn_even_when_first_click_is_after_goal_advances() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store.sqlite");
        enqueue(
            &path,
            NewQueueJob {
                job_id: "job",
                target_thread_id: "thread",
                channel_id: 2,
                owner_user_id: Some(1),
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
        let choice = create_busy_choice(
            &path,
            NewBusyChoice {
                owner_user_id: 1,
                channel_id: 2,
                target_thread_id: Some("thread"),
                prompt: "steer",
                allow_steer: false,
                now: 1.0,
                time_to_live: 100.0,
            },
        )
        .unwrap();
        bind(&path, &choice, "thread", None, Some("job")).unwrap();
        assert_eq!(resolve(&path, &choice, "thread").unwrap(), None);
        mark_running(&path, "job", "first", 7).unwrap();
        assert!(mark_goal_waiting(&path, "job", "first", 7).unwrap());
        assert!(attach_goal_turn(&path, "thread", "later", 7).unwrap());
        assert_eq!(
            resolve(&path, &choice, "thread").unwrap().as_deref(),
            Some("first")
        );
    }
    #[test]
    fn original_turn_cannot_be_rebound_to_a_later_turn_or_another_thread() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store.sqlite");
        let choice = create_busy_choice(
            &path,
            NewBusyChoice {
                owner_user_id: 1,
                channel_id: 2,
                target_thread_id: Some("thread"),
                prompt: "steer",
                allow_steer: true,
                now: 1.0,
                time_to_live: 100.0,
            },
        )
        .unwrap();
        bind(&path, &choice, "thread", Some("first"), None).unwrap();
        bind(&path, &choice, "thread", Some("later"), None).unwrap();
        assert_eq!(
            resolve(&path, &choice, "thread").unwrap().as_deref(),
            Some("first")
        );
        assert_eq!(resolve(&path, &choice, "foreign").unwrap(), None);
        assert_eq!(resolve(&path, "legacy-unbound", "thread").unwrap(), None);
    }
}
