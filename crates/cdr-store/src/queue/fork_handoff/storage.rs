use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{AppServerForkHandoff, AppServerForkHandoffError};

pub(super) const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS \
    codex_thread_fork_handoffs (\
      handoff_id TEXT PRIMARY KEY, \
      ambiguous_job_id TEXT UNIQUE, \
      source_thread_id TEXT NOT NULL UNIQUE, \
      expected_generation INTEGER NOT NULL, \
      discord_channel_id INTEGER NOT NULL, \
      discord_thread_id INTEGER NOT NULL, \
      quarantine_reason TEXT NOT NULL, \
      last_fork_error TEXT NOT NULL DEFAULT '', \
      fork_failure_ambiguous INTEGER NOT NULL DEFAULT 0, \
      observed_target_thread_id TEXT, \
      target_thread_id TEXT UNIQUE, \
      completed_generation INTEGER, \
      created_at REAL NOT NULL, \
      completed_at REAL, \
      CHECK ((target_thread_id IS NULL AND completed_generation IS NULL AND completed_at IS NULL) \
          OR (target_thread_id IS NOT NULL AND completed_generation IS NOT NULL \
              AND completed_at IS NOT NULL)), \
      CHECK (target_thread_id IS NULL OR target_thread_id = observed_target_thread_id)\
    )";

const COLUMNS: &str = "handoff_id, ambiguous_job_id, source_thread_id, expected_generation, \
    discord_channel_id, discord_thread_id, quarantine_reason, last_fork_error, \
    fork_failure_ambiguous, observed_target_thread_id, target_thread_id, completed_generation";

pub(super) fn ensure_table(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute(CREATE_TABLE, [])?;
    let mut statement = connection.prepare("PRAGMA table_info(codex_thread_fork_handoffs)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    if !columns
        .iter()
        .any(|column| column == "observed_target_thread_id")
    {
        connection.execute(
            "ALTER TABLE codex_thread_fork_handoffs \
             ADD COLUMN observed_target_thread_id TEXT",
            [],
        )?;
    }
    if !columns.iter().any(|column| column == "last_fork_error") {
        connection.execute(
            "ALTER TABLE codex_thread_fork_handoffs \
             ADD COLUMN last_fork_error TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !columns
        .iter()
        .any(|column| column == "fork_failure_ambiguous")
    {
        connection.execute(
            "ALTER TABLE codex_thread_fork_handoffs \
             ADD COLUMN fork_failure_ambiguous INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    connection.execute(
        "DROP INDEX IF EXISTS codex_thread_fork_handoffs_observed_target",
        [],
    )?;
    Ok(())
}

pub(super) fn by_id(
    connection: &Connection,
    handoff_id: &str,
) -> Result<Option<AppServerForkHandoff>, rusqlite::Error> {
    select_one(
        connection,
        &format!("SELECT {COLUMNS} FROM codex_thread_fork_handoffs WHERE handoff_id = ?"),
        handoff_id,
    )
}

pub(super) fn by_source(
    connection: &Connection,
    source_thread_id: &str,
) -> Result<Option<AppServerForkHandoff>, rusqlite::Error> {
    select_one(
        connection,
        &format!("SELECT {COLUMNS} FROM codex_thread_fork_handoffs WHERE source_thread_id = ?"),
        source_thread_id,
    )
}

pub(super) fn by_ambiguous_job(
    connection: &Connection,
    job_id: &str,
) -> Result<Option<AppServerForkHandoff>, rusqlite::Error> {
    select_one(
        connection,
        &format!("SELECT {COLUMNS} FROM codex_thread_fork_handoffs WHERE ambiguous_job_id = ?"),
        job_id,
    )
}

pub(super) fn managed_target(
    connection: &Connection,
    thread_id: &str,
) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_thread_fork_handoffs \
         WHERE target_thread_id = ? AND completed_at IS NOT NULL)",
        [thread_id],
        |row| row.get(0),
    )
}

pub(super) fn insert(
    transaction: &Transaction<'_>,
    handoff: &AppServerForkHandoff,
    now: f64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO codex_thread_fork_handoffs (handoff_id, ambiguous_job_id, \
         source_thread_id, expected_generation, discord_channel_id, discord_thread_id, \
         quarantine_reason, last_fork_error, fork_failure_ambiguous, \
         observed_target_thread_id, target_thread_id, completed_generation, \
         created_at, completed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, '', 0, NULL, NULL, NULL, ?, NULL)",
        params![
            handoff.handoff_id,
            handoff.ambiguous_job_id,
            handoff.source_thread_id,
            handoff.expected_generation,
            handoff.discord_channel_id,
            handoff.discord_thread_id,
            handoff.quarantine_reason,
            now,
        ],
    )?;
    Ok(())
}

pub(super) fn record_fork_failure(
    transaction: &Transaction<'_>,
    handoff_id: &str,
    error: &str,
    ambiguous: bool,
) -> Result<usize, rusqlite::Error> {
    transaction.execute(
        "UPDATE codex_thread_fork_handoffs SET last_fork_error = ?, \
         fork_failure_ambiguous = CASE WHEN fork_failure_ambiguous != 0 OR ? != 0 \
             THEN 1 ELSE 0 END \
         WHERE handoff_id = ? AND target_thread_id IS NULL \
         AND observed_target_thread_id IS NULL",
        params![error, i64::from(ambiguous), handoff_id],
    )
}

pub(super) fn mark_target_observed(
    transaction: &Transaction<'_>,
    handoff_id: &str,
    target_thread_id: &str,
) -> Result<usize, rusqlite::Error> {
    transaction.execute(
        "UPDATE codex_thread_fork_handoffs SET observed_target_thread_id = ? \
         WHERE handoff_id = ? AND observed_target_thread_id IS NULL \
         AND target_thread_id IS NULL",
        params![target_thread_id, handoff_id],
    )
}

pub(super) fn mark_completed(
    transaction: &Transaction<'_>,
    handoff_id: &str,
    target_thread_id: &str,
    generation: i64,
    now: f64,
) -> Result<(), AppServerForkHandoffError> {
    let updated = transaction.execute(
        "UPDATE codex_thread_fork_handoffs SET target_thread_id = ?, \
         completed_generation = ?, completed_at = ? \
         WHERE handoff_id = ? AND target_thread_id IS NULL \
         AND observed_target_thread_id = ?",
        params![
            target_thread_id,
            generation,
            now,
            handoff_id,
            target_thread_id
        ],
    )?;
    if updated != 1 {
        return Err(AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff_id.to_owned(),
        });
    }
    Ok(())
}

pub(super) fn cancel_unobserved(
    transaction: &Transaction<'_>,
    handoff_id: &str,
) -> Result<usize, rusqlite::Error> {
    transaction.execute(
        "DELETE FROM codex_thread_fork_handoffs WHERE handoff_id = ? \
         AND observed_target_thread_id IS NULL AND target_thread_id IS NULL",
        [handoff_id],
    )
}

pub(super) fn completed_target_for_source(
    connection: &Connection,
    source_thread_id: &str,
) -> Result<Option<String>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT target_thread_id FROM codex_thread_fork_handoffs \
             WHERE source_thread_id = ? AND completed_at IS NOT NULL",
            [source_thread_id],
            |row| row.get(0),
        )
        .optional()
}

fn select_one(
    connection: &Connection,
    sql: &str,
    value: &str,
) -> Result<Option<AppServerForkHandoff>, rusqlite::Error> {
    connection
        .query_row(sql, [value], |row| {
            Ok(AppServerForkHandoff {
                handoff_id: row.get(0)?,
                ambiguous_job_id: row.get(1)?,
                source_thread_id: row.get(2)?,
                expected_generation: row.get(3)?,
                discord_channel_id: row.get(4)?,
                discord_thread_id: row.get(5)?,
                quarantine_reason: row.get(6)?,
                last_fork_error: row.get(7)?,
                fork_failure_ambiguous: row.get(8)?,
                observed_target_thread_id: row.get(9)?,
                target_thread_id: row.get(10)?,
                completed_generation: row.get(11)?,
            })
        })
        .optional()
}
