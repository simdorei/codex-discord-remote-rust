//! Carry the admission-time lifecycle target into the operation itself.
use super::{ActionContext, ActionError, ActionExecutor, ActionResult, id_i64};
use crate::{
    command_plan::CommandAction, message_plan::MessagePlan, queue_runner::TurnBackend,
    settings_binding::SettingsBinding,
};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn validate_lifecycle_binding(
        &self,
        binding: Option<&SettingsBinding>,
        channel: u64,
    ) -> Result<(), ActionError> {
        if let Some(binding) = binding {
            self.settings_resolver()
                .validate_lifecycle(binding, channel)?;
        }
        Ok(())
    }

    pub(super) async fn execute_lifecycle_with_ingress(
        &self,
        action: CommandAction,
        context: ActionContext,
        key: &str,
    ) -> Result<ActionResult, ActionError> {
        let record = cdr_store::ingress::get(&self.mirror_db, key)?
            .ok_or_else(|| invalid("admission record is missing"))?;
        let event = context.discord_message_id.map(id_i64).transpose()?;
        let command = serde_json::to_value(&action)
            .map_err(|error| ActionError::Invalid(error.to_string()))?;
        let plan = serde_json::to_value(MessagePlan::Execute(action.clone()))
            .map_err(|error| ActionError::Invalid(error.to_string()))?;
        if record.kind != cdr_store::ingress::IngressKind::Message
            || record.channel_id != id_i64(context.channel_id)?
            || record.owner_user_id != id_i64(context.user_id)?
            || event.is_none()
            || record.event_id != event
            || record.source_message_id != event
            || record.state != "executing"
            || record.phase != "processing"
            || record.owner_id.is_some()
            || record.payload.get("version") != Some(&serde_json::json!(1))
            || record.payload.get("plan") != Some(&plan)
        {
            return Err(invalid("original command/user/channel envelope differs"));
        }
        let binding: SettingsBinding = serde_json::from_value(
            record
                .payload
                .get("lifecycle_binding")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .map_err(|_| invalid("original target was not frozen; legacy request remains preserved"))?;
        if binding.command != command
            || record.target_thread_id.as_deref() != Some(binding.target.as_str())
        {
            return Err(invalid("admitted command or target identity differs"));
        }
        self.settings_resolver()
            .validate_lifecycle(&binding, context.channel_id)?;
        match action {
            CommandAction::Archive { reference } => {
                self.archive_bound(context, reference.as_deref(), Some(&binding))
                    .await
            }
            CommandAction::Resume { reference } => {
                self.resume_bound(context.channel_id, reference.as_deref(), Some(&binding))
                    .await
            }
            _ => unreachable!("only lifecycle operations enter this custody boundary"),
        }
    }
}

fn invalid(reason: &str) -> ActionError {
    ActionError::Invalid(format!(
        "lifecycle {reason}; no replacement target or lifecycle RPC will be used"
    ))
}
