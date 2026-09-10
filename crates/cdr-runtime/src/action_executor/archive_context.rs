//! Prove the current archive envelope before excluding it from pending ingress.
use super::{ActionContext, ActionError, ActionExecutor, id_i64};
use crate::{
    command_plan::CommandAction,
    message_plan::MessagePlan,
    prefix_plan::{PrefixAction, plan_prefix},
    queue_runner::TurnBackend,
};
use cdr_store::ingress::{IngressKind, by_origin};

#[cfg(test)]
#[path = "lifecycle_route_contract.rs"]
mod route_contract;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn archive_own_request(
        &self,
        context: ActionContext,
        reference: Option<&str>,
    ) -> Result<Option<String>, ActionError> {
        let Some(event) = context.discord_message_id else {
            return Ok(None);
        };
        let event = id_i64(event)?;
        let row = by_origin(&self.mirror_db, event)?.ok_or_else(invalid_envelope)?;
        let expected = serde_json::to_value(MessagePlan::Execute(CommandAction::Archive {
            reference: reference.map(str::to_owned),
        }))
        .map_err(|error| ActionError::Invalid(error.to_string()))?;
        let command_text = row
            .payload
            .get("content")
            .and_then(serde_json::Value::as_str)
            .and_then(|content| content.trim().strip_prefix('!'));
        let correct_command = command_text.and_then(|text| plan_prefix(text).ok())
            == Some(PrefixAction::Archive {
                reference: reference.map(str::to_owned),
            });
        if row.kind != IngressKind::Message
            || row.source_message_id != Some(event)
            || row.channel_id != id_i64(context.channel_id)?
            || row.owner_user_id != id_i64(context.user_id)?
            || row.state != "executing"
            || row.phase != "processing"
            || row.owner_id.is_some()
            || row.payload.get("version") != Some(&serde_json::json!(1))
            || row.payload.get("plan") != Some(&expected)
            || !correct_command
        {
            return Err(invalid_envelope());
        }
        Ok(Some(row.ingress_id))
    }
}

fn invalid_envelope() -> ActionError {
    ActionError::Invalid(
        "archive cannot verify its original command/user/channel envelope; no archive was sent"
            .into(),
    )
}
