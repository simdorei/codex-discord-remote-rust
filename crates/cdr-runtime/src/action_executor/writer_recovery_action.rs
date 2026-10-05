use super::recovery_custody::RecoveryGuard;
use super::{ActionContext, ActionError, ActionExecutor, ActionResult, immediate};
use crate::{command_plan::CommandAction, queue_runner::TurnBackend};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn recover_writer(
        &self,
        context: ActionContext,
        reference: Option<&str>,
    ) -> Result<ActionResult, ActionError> {
        let guard = self.freeze_recovery(
            context.channel_id,
            &CommandAction::Recover {
                reference: reference.map(str::to_owned),
            },
        )?;
        self.recover_writer_bound(context, &guard).await
    }

    pub(super) async fn recover_writer_bound(
        &self,
        context: ActionContext,
        guard: &RecoveryGuard,
    ) -> Result<ActionResult, ActionError> {
        // Do not wait behind a stuck turn. The store's transactional cancellation
        // boundary races execution claims safely, and the controller pins the OS owner.
        guard.check()?;
        let root = std::env::current_dir()?;
        #[cfg(test)]
        let root = self.recovery_root.clone().unwrap_or(root);
        let home = self
            .state_db
            .parent()
            .ok_or_else(|| ActionError::Invalid("Codex home is missing".into()))?;
        let report = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            crate::writer_recovery::run_tools_checked(
            crate::writer_recovery::Target {
                root: &root,
                codex_home: home,
                database: &self.mirror_db,
                thread: guard.target(),
                channel: super::id_i64(context.channel_id)?,
                user: super::id_i64(context.user_id)?,
            },
            false,
            &|| guard.check().map_err(|e| e.to_string()),
            &|db| guard.check_in(db),
            ),
        )
            .await
            .map_err(|_| {
                ActionError::Invalid(
                    "recovery handoff timed out after 20s; cancellation and a recovery operation may already be committed; inspect the recovery receipt before any further action; the original request will not be replayed automatically"
                        .into(),
                )
            })?
            .map_err(ActionError::Invalid)?;
        Ok(immediate(crate::writer_recovery::message(&report)))
    }
}
