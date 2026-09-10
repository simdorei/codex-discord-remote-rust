use crate::{
    action_executor::{ActionError, ActionExecutor, id_i64},
    queue_runner::TurnBackend,
};
use cdr_codex_state::{CodexThreadStore, normalize_workspace_path, strip_windows_extended_prefix};
use cdr_store::mapping::NewThreadOrigin;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn freeze_new_context(
        &self,
        ingress: &cdr_store::ingress::StoredIngress,
        context: crate::action_executor::ActionContext,
        generation: i64,
    ) -> Result<Option<String>, ActionError> {
        let origin: NewThreadOrigin = serde_json::from_value(ingress.payload["new_origin"].clone())
            .map_err(|error| {
                self.hold_new_thread(
                    ingress,
                    &ActionError::Invalid(format!(
                        "new origin snapshot is missing or invalid: {error}"
                    )),
                    true,
                )
            })?;
        let cwd = self
            .new_thread_cwd(&origin)
            .map_err(|error| self.hold_new_thread(ingress, &error, true))?;
        if let Some(cwd) = &cwd {
            for key in [
                origin.mapped_project.as_deref(),
                origin.project.as_deref(),
                origin.parent_project.as_deref(),
            ]
            .into_iter()
            .flatten()
            .filter(|key| *key != "codex:chats" && !key.starts_with("projectless:"))
            {
                if normalize_workspace_path(key) != normalize_workspace_path(cwd) {
                    return Err(self.hold_new_thread(
                        ingress,
                        &ActionError::Invalid(
                            "Codex working directory differs from the frozen Discord project; no thread/start permitted".into(),
                        ),
                        true,
                    ));
                }
            }
        }
        if self.mirror_sync.get().is_some() && cwd.is_none() {
            return Err(self.hold_new_thread(
                ingress,
                &ActionError::Invalid("new request has no verified originating project".into()),
                true,
            ));
        }
        cdr_store::ingress::record_new_creation(
            &self.mirror_db,
            &ingress.ingress_id,
            generation,
            cwd.as_deref(),
            id_i64(context.channel_id)?,
            super::journal::unix_now()?,
        )
        .map_err(|error| self.hold_new_thread(ingress, &error.into(), true))?;
        Ok(cwd)
    }

    fn new_thread_cwd(&self, origin: &NewThreadOrigin) -> Result<Option<String>, ActionError> {
        if let Some(id) = &origin.target {
            let thread = CodexThreadStore::open(&self.state_db)?
                .load_thread(id, false)?
                .ok_or_else(|| {
                    ActionError::Invalid(format!("cannot resolve project of mapped thread {id}"))
                })?;
            return checked_cwd(&thread.cwd).map(Some);
        }
        if let Some(key) = &origin.project {
            if key == "codex:chats" || key.starts_with("projectless:") {
                let threads = CodexThreadStore::open(&self.state_db)?.load_recent_threads(0)?;
                let cwd = threads
                    .iter()
                    .find(|t| origin.chat_targets.contains(&t.id))
                    .ok_or_else(|| {
                        ActionError::Invalid("chat project has no known working directory".into())
                    })?;
                return checked_cwd(&cwd.cwd).map(Some);
            }
            return checked_cwd(key).map(Some);
        }
        Ok(None)
    }
}

fn checked_cwd(cwd: &str) -> Result<String, ActionError> {
    let cwd = strip_windows_extended_prefix(cwd);
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(ActionError::Invalid(format!(
            "project directory is unavailable: {cwd}"
        )));
    }
    Ok(cwd)
}
