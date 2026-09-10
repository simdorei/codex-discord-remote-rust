use super::{ActionError, ActionExecutor};
use crate::queue_runner::TurnBackend;
use cdr_store::prompt_intake::list_prompt_intakes;
use cdr_store::queue::{QueueJobState, list};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn runners_message(&self) -> Result<String, ActionError> {
        let jobs = list(&self.mirror_db)?;
        let final_pending = cdr_store::delivery::list_pending(&self.mirror_db)?.len();
        let sends_unconfirmed = cdr_store::delivery_receipt::unknown_count(&self.mirror_db)?;
        let sends_rejected = cdr_store::delivery_receipt::blocked_count(&self.mirror_db)?;
        let intakes = list_prompt_intakes(&self.mirror_db)?;
        let pending = jobs
            .iter()
            .filter(|job| job.state == QueueJobState::Pending)
            .count();
        let starting = jobs
            .iter()
            .filter(|job| job.state == QueueJobState::Starting)
            .count();
        let running = jobs
            .iter()
            .filter(|job| job.state == QueueJobState::Running)
            .count();
        let quarantined = jobs
            .iter()
            .filter(|job| job.state == QueueJobState::Quarantined)
            .count();
        let intake_backoff = intakes
            .iter()
            .filter(|intake| !intake.last_error.is_empty())
            .count();
        Ok(format!(
            "Codex runners\npending: {pending}\nstarting: {starting}\nrunning: {running}\nquarantined: {quarantined}\nfinal_pending: {final_pending}\nsends_unconfirmed: {sends_unconfirmed} (not automatically resent)\nsends_rejected: {sends_rejected} (requires correction; no automatic retry)\nrecoverable_intakes: {}\nintake_backoff: {intake_backoff}",
            intakes.len()
        ))
    }
}
