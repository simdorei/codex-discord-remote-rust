use super::{ActionError, ActionExecutor};
use crate::queue_runner::TurnBackend;
use rusqlite::params;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn runner_target_summary(&self, channel: u64) -> Result<String, ActionError> {
        let (target, source) = match self.target(channel) {
            Ok(value) => value,
            Err(ActionError::NoTarget) => {
                return Ok(format!(
                    "Current target lookup failed: {}. Global counts and your saved requests remain available; no target was substituted.",
                    ActionError::NoTarget
                ));
            }
            Err(error) => return Err(error),
        };
        let owned = match self.verified_control_turn(channel, &target, None).await {
            Ok((turn, generation)) => {
                format!("owned_active_turn: {turn}\nconnection_generation: {generation}")
            }
            Err(error @ (ActionError::Invalid(_) | ActionError::MissingAppServer)) => {
                format!("owned_active_turn: unknown\nactive_check: {error}")
            }
            Err(error) => return Err(error),
        };
        let db = cdr_store::schema::open_initialized(&self.mirror_db)?;
        let counts: (i64,i64,i64,i64,i64,i64,i64) = db.query_row("SELECT
            (SELECT COUNT(*) FROM codex_turn_queue WHERE target_thread_id=?1 AND state='pending'),
            (SELECT COUNT(*) FROM codex_turn_queue WHERE target_thread_id=?1 AND state='starting'),
            (SELECT COUNT(*) FROM codex_turn_queue WHERE target_thread_id=?1 AND state='running'
             AND NOT COALESCE(instr(turn_id,'cdr-quarantined:')=1 AND instr(last_error,'[cdr-rust:app-server-fork-quarantine:v1] ')=1,0)),
            (SELECT COUNT(*) FROM codex_prompt_intakes WHERE target_thread_id=?1),
            (SELECT COUNT(*) FROM discord_ingress_journal WHERE target_thread_id=?1 AND state='held'),
            (SELECT COUNT(*) FROM discord_ingress_journal WHERE target_thread_id=?1 AND state IN ('staged','acknowledged','executing') AND owner_id IS NULL),
            (SELECT COUNT(*) FROM codex_turn_queue WHERE target_thread_id=?1 AND (state='quarantined' OR
             (state='running' AND instr(turn_id,'cdr-quarantined:')=1 AND instr(last_error,'[cdr-rust:app-server-fork-quarantine:v1] ')=1)))",
            params![target], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)))
            .map_err(cdr_store::StoreError::from)?;
        Ok(format!(
            "Current target work (stored counts and resident ownership check; no remote probe)\ntarget: {target}\nsource: {source}\n{owned}\nqueued: {}\nstarting: {}\nrunning_records: {}\nintake: {}\nheld_ingress: {}\nunowned_ingress: {}\nquarantined_records: {}",
            counts.0, counts.1, counts.2, counts.3, counts.4, counts.5, counts.6
        ))
    }
}
