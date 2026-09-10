//! Explicit reference, current room, and global selection have distinct meanings.
use super::{ActionError, ActionExecutor};
use crate::queue_runner::TurnBackend;
use cdr_codex_state::{CodexThreadStore, ThreadInfo, ThreadResolveError, resolve_thread_ref};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn resolve_reference(
        &self,
        reference: &str,
        archived: bool,
    ) -> Result<ThreadInfo, ActionError> {
        let store = CodexThreadStore::open(&self.state_db)?;
        let reference = reference.trim();
        if let Some(thread) = store.load_thread(reference, archived)? {
            return Ok(thread);
        }
        // Reference disambiguation must inspect all candidates, not a display page.
        let threads = if archived {
            store.load_archived_threads(0)?
        } else {
            store.load_recent_threads(0)?
        };
        let selected = self.bridge_state.selected_thread_id()?;
        Ok(resolve_thread_ref(&threads, reference, selected.as_deref(), archived)?.clone())
    }

    pub(super) fn resolve_thread(
        &self,
        channel_id: u64,
        reference: Option<&str>,
    ) -> Result<ThreadInfo, ActionError> {
        if let Some(reference) = reference {
            return self.resolve_reference(reference, false);
        }
        let target = self.target(channel_id)?.0;
        CodexThreadStore::open(&self.state_db)?
            .load_thread(&target, false)?
            .ok_or_else(|| ThreadResolveError::NotFound(target).into())
    }
}
