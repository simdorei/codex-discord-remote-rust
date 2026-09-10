use crate::{StoreError, schema::open_initialized};
use std::path::Path;

/// Retain old copy-only handoffs as audit history, not live routing instructions.
/// Never release an ambiguous request-start hold or change a room/job destination.
pub fn retire_copy_only_handoffs(path: &Path) -> Result<usize, StoreError> {
    let mut connection = open_initialized(path)?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    super::storage::ensure_table(&transaction)?;
    transaction.execute_batch("CREATE TABLE IF NOT EXISTS codex_exact_thread_routing (enabled INTEGER NOT NULL CHECK(enabled=1));
        INSERT INTO codex_exact_thread_routing SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM codex_exact_thread_routing);")?;
    let ids = {
        let mut query = transaction.prepare(
            "SELECT handoff_id FROM codex_thread_fork_handoffs h
             WHERE ambiguous_job_id IS NULL
             AND NOT EXISTS (SELECT 1 FROM codex_turn_queue q
               WHERE q.target_thread_id IN (h.source_thread_id, h.observed_target_thread_id, h.target_thread_id)
               AND q.state IN ('starting','running','quarantined'))
             AND NOT EXISTS (SELECT 1 FROM codex_dead_generation_holds d
               WHERE d.target_thread_id IN (h.source_thread_id, h.observed_target_thread_id, h.target_thread_id))"
        )?;
        query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    if !ids.is_empty() {
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS codex_retired_fork_handoffs AS SELECT * FROM codex_thread_fork_handoffs WHERE 0;
            CREATE UNIQUE INDEX IF NOT EXISTS codex_retired_fork_id ON codex_retired_fork_handoffs(handoff_id);")?;
        for id in &ids {
            transaction.execute("INSERT INTO codex_retired_fork_handoffs SELECT * FROM codex_thread_fork_handoffs WHERE handoff_id=?", [id])?;
            transaction.execute(
                "DELETE FROM codex_thread_fork_handoffs WHERE handoff_id=?",
                [id],
            )?;
        }
    }
    transaction.commit()?;
    Ok(ids.len())
}

pub(super) fn enabled(connection: &rusqlite::Connection) -> Result<bool, StoreError> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='codex_exact_thread_routing')", [], |r| r.get(0))?)
}
