use super::{ActionError, ActionExecutor, ActionResult, immediate};
use crate::queue_runner::TurnBackend;
use cdr_codex_state::CodexThreadStore;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn context(
        &self,
        channel_id: u64,
        all_threads: bool,
        refresh: bool,
        limit: u32,
    ) -> Result<ActionResult, ActionError> {
        let store = CodexThreadStore::open(&self.state_db)?;
        let threads = if all_threads {
            store.load_recent_threads(limit)?
        } else {
            vec![self.resolve_thread(channel_id, None)?]
        };
        let original = (!all_threads).then(|| threads[0].id.clone());
        let text = crate::context_view::render(threads, refresh, limit as usize)
            .await
            .map_err(ActionError::Invalid)?;
        if let Some(original) = original
            && self.target(channel_id)?.0 != original
        {
            return Err(ActionError::Invalid(
                "context target changed during read; no current-room snapshot confirmed".into(),
            ));
        }
        Ok(immediate(text))
    }
}
