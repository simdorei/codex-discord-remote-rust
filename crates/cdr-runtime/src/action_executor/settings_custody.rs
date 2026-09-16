use super::{ActionContext, ActionError, ActionExecutor, ActionResult};
use crate::{
    command_plan::CommandAction,
    queue_runner::TurnBackend,
    settings_binding::{SettingsBinding, is_settings_mutation},
};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(crate) async fn execute_with_ingress_context(
        &self,
        action: CommandAction,
        context: ActionContext,
        key: &str,
    ) -> Result<ActionResult, ActionError> {
        if matches!(
            action,
            CommandAction::Archive { .. } | CommandAction::Resume { .. }
        ) {
            return self
                .execute_lifecycle_with_ingress(action, context, key)
                .await;
        }
        if !is_settings_mutation(&action) {
            return self.execute_with_context(action, context).await;
        }
        let record = cdr_store::ingress::get(&self.mirror_db, key)?
            .ok_or_else(|| ActionError::Invalid("settings admission record is missing".into()))?;
        if u64::try_from(record.channel_id).ok() != Some(context.channel_id)
            || u64::try_from(record.owner_user_id).ok() != Some(context.user_id)
        {
            return Err(ActionError::Invalid(
                "settings admission owner or channel differs".into(),
            ));
        }
        let binding: SettingsBinding = serde_json::from_value(
            record
                .payload
                .get("settings_binding")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .map_err(|_| {
            ActionError::Invalid(
                "settings target was not frozen before admission; request will not be redirected"
                    .into(),
            )
        })?;
        if record.target_thread_id.as_deref() != Some(binding.target.as_str()) {
            return Err(ActionError::Invalid(
                "settings admission target identity differs".into(),
            ));
        }
        if serde_json::to_value(&action).map_err(|error| ActionError::Invalid(error.to_string()))?
            != binding.command
        {
            return Err(ActionError::Invalid(
                "settings command differs from its admitted envelope".into(),
            ));
        }
        match action {
            CommandAction::Settings { reference, model, effort, speed } => {
                self.settings(
                    context.channel_id,
                    reference.as_deref(),
                    model.as_deref(),
                    effort.as_deref(),
                    speed.as_deref(),
                    Some(&binding),
                ).await
            }
            CommandAction::AutoReserve { enabled, .. } => {
                self.auto_reserve_setting(
                    context.channel_id,
                    Some(binding.target.as_str()),
                    enabled,
                    Some(&binding),
                ).await
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn validate_settings_route(
        &self,
        channel: u64,
        reference: Option<&str>,
        thread: &str,
        binding: Option<&SettingsBinding>,
    ) -> Result<(), ActionError> {
        if let Some(binding) = binding {
            self.settings_resolver().validate(binding, channel)
        } else if reference.is_none() && self.target(channel)?.0 != thread {
            Err(ActionError::Invalid(
                "settings target changed; no replacement target will be used".into(),
            ))
        } else {
            Ok(())
        }
    }
}
