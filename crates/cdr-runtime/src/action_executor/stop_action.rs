use super::{ActionContext, ActionError, ActionExecutor, ActionResult, id_i64, immediate};
use crate::{
    queue_runner::TurnBackend,
    settings_binding::{SettingsBinding, SettingsRoute},
};
use cdr_store::ingress::{
    StoredIngress,
    stop::{StopScope, accept_unresolved},
};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn stop_bound(
        &self,
        context: ActionContext,
        binding: &SettingsBinding,
        ingress: Option<&StoredIngress>,
    ) -> Result<ActionResult, ActionError> {
        let resolver = self.settings_resolver();
        let scope = StopScope {
            target: &binding.target,
            channel: id_i64(context.channel_id)?,
            owner: id_i64(context.user_id)?,
        };
        let frozen = serde_json::to_value(binding)
            .map_err(|error| ActionError::Invalid(error.to_string()))?;
        if let Some(server) = &self.server {
            let receipt = cdr_store::ingress::stop::control::accept_running(
                &self.mirror_db,
                scope,
                &frozen,
                ingress,
                (server.instance_id(), id_i64(server.generation())?),
                || {
                    resolver
                        .validate_selected_snapshot(binding)
                        .map_err(|error| cdr_store::StoreError::Integrity(error.to_string()))
                },
            )?;
            if let Some(receipt) = receipt {
                return Ok(immediate(format!(
                    "Stop accepted for {}.\noperation_id: {}\nExecution end is not confirmed; the original turn will be checked separately. Original requests will not be replayed automatically.",
                    binding.target, receipt.operation_id
                )));
            }
        }
        let receipt = accept_unresolved(&self.mirror_db, scope, &frozen, ingress, || {
            resolver
                .validate_selected_snapshot(binding)
                .map_err(|error| cdr_store::StoreError::Integrity(error.to_string()))
        })?;
        if let Some(receipt) = receipt {
            return Ok(immediate(format!(
                "Stop accepted for {}.\nOriginal local requests held (queued, preparing, running or unresolved): {}\nUnowned original requests held: {}\nExecution end is not confirmed; original requests will not be replayed automatically.",
                binding.target,
                receipt.jobs.len(),
                receipt.ingresses.len()
            )));
        }

        // Running controls retain exact active-turn verification, never resume/fork.
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let _control = self.control_lock(&binding.target).await?;
        resolver.validate_lifecycle(binding, context.channel_id)?;
        let (turn, generation) = if binding.route == SettingsRoute::Explicit {
            self.verified_owned_turn(&binding.target, None).await?
        } else {
            self.verified_control_turn(context.channel_id, &binding.target, None)
                .await?
        };
        server
            .execute(
                cdr_app_server::requests::interrupt_turn(&binding.target, &turn),
                Some(generation),
            )
            .await?;
        Ok(immediate(format!(
            "Stop request submitted for {}.",
            binding.target
        )))
    }
}
