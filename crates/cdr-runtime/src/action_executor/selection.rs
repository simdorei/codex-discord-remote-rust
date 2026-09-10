use cdr_codex_state::CodexThreadStore;
use cdr_store::mapping::mirrored_thread_id;

use super::format::workspace_name;
use super::{ActionError, ActionExecutor, ActionResult, id_i64, immediate};
use crate::queue_runner::TurnBackend;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn thread_list(
        &self,
        limit: u32,
        archived: bool,
    ) -> Result<ActionResult, ActionError> {
        let store = CodexThreadStore::open(&self.state_db)?;
        let threads = if archived {
            store.load_archived_threads(0)?
        } else {
            store.load_recent_threads(0)?
        };
        if threads.is_empty() {
            return Ok(immediate(if archived {
                "No archived Codex threads found in the local state DB."
            } else {
                "No Codex threads found in the local state DB."
            }));
        }
        let selected = self.bridge_state.selected_thread_id()?;
        let observations = if archived {
            std::collections::BTreeMap::new()
        } else {
            super::list_view::states(self.server.as_deref(), &threads, limit).await
        };
        let text = super::list_view::render(threads, selected, limit, archived, observations)
            .await
            .map_err(ActionError::Invalid)?;
        Ok(immediate(text))
    }

    pub(super) fn select(&self, reference: &str) -> Result<String, ActionError> {
        let thread = self.resolve_reference(reference, false)?;
        self.bridge_state.set_selected_thread_id(Some(&thread.id))?;
        Ok(format!(
            "Selected Codex thread\nthread_id: {}\nworkspace: {}\ntitle: {}",
            thread.id,
            workspace_name(&thread),
            thread.title
        ))
    }

    pub(super) fn where_message(&self, channel_id: u64) -> Result<String, ActionError> {
        let (thread_id, source) = self.target(channel_id)?;
        let thread = self.resolve_thread(channel_id, None)?;
        if thread.id != thread_id {
            return Err(ActionError::Invalid(
                "where target changed during lookup".into(),
            ));
        }
        let mapping = if source == "mirror" {
            "mapped to this Discord room"
        } else {
            "unmapped; using global selected thread (not a room mapping)"
        };
        Ok(format!(
            "Codex target\nsource: {source}\nthread_id: {thread_id}\ndiscord_mapping: {mapping}\ntitle: {}\ncwd: {}",
            thread.title, thread.cwd
        ))
    }

    pub(super) fn target(&self, channel_id: u64) -> Result<(String, &'static str), ActionError> {
        if let Some(thread) = mirrored_thread_id(&self.mirror_db, Some(id_i64(channel_id)?))? {
            return Ok((thread, "mirror"));
        }
        self.bridge_state
            .selected_thread_id()?
            .map(|thread| (thread, "selected"))
            .ok_or(ActionError::NoTarget)
    }
}
