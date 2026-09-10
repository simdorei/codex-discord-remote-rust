use super::{ActionError, ActionExecutor, id_i64};
use crate::queue_runner::TurnBackend;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn retract_prompt(
        &self,
        channel_id: u64,
        user_id: u64,
        reference: Option<&str>,
    ) -> Result<String, ActionError> {
        let (target, source) = if let Some(reference) = reference {
            (self.resolve_reference(reference, false)?.id, "explicit")
        } else {
            self.target(channel_id)?
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs_f64();
        let removed = cdr_store::queue::cancel_latest_pending_on_route(
            &self.mirror_db,
            &target,
            id_i64(channel_id)?,
            id_i64(user_id)?,
            now,
            source == "mirror",
        )?;
        Ok(removed.map_or_else(
            || format!("No unstarted request found for {target}."),
            |job| format!("Retracted unstarted request {job} for {target}."),
        ))
    }
}
