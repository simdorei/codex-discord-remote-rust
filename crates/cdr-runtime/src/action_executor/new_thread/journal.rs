use std::time::{SystemTime, UNIX_EPOCH};

use cdr_store::ingress::{IngressKind, NewIngress, StoredIngress, admit, by_origin, hold};
use serde_json::json;
use uuid::Uuid;

use crate::action_executor::{ActionContext, ActionError, ActionExecutor, id_i64};
use crate::queue_runner::TurnBackend;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn admit_new_thread(
        &self,
        context: ActionContext,
        prompt: &str,
    ) -> Result<StoredIngress, ActionError> {
        if prompt.trim().is_empty() {
            return Err(ActionError::Invalid(
                "new request prompt must not be blank".into(),
            ));
        }
        let event_id = context.discord_message_id.map(id_i64).transpose()?;
        if let Some(event_id) = event_id
            && let Some(existing) = by_origin(&self.mirror_db, event_id)?
        {
            validate_canonical_record(&existing, context, prompt)?;
            return Ok(existing);
        }
        admit_after_lookup_miss(&self.mirror_db, context, prompt)
    }

    pub(super) fn hold_new_thread(
        &self,
        ingress: &StoredIngress,
        error: &ActionError,
        not_executed: bool,
    ) -> ActionError {
        // Persist a public-safe reason; return the original failure to the
        // caller without copying remote error content into recovery notices.
        let recording = unix_now().and_then(|now| {
            hold(
                &self.mirror_db,
                &ingress.ingress_id,
                "new-thread creation needs manual review; automatic recreation is disabled",
                not_executed,
                now,
            )
            .map_err(ActionError::from)
        });
        let message = format!(
            "request {} is preserved for manual review; no automatic thread/start retry: {error}",
            ingress.ingress_id
        );
        ActionError::Invalid(match recording {
            Ok(()) => message,
            Err(recording) => {
                format!("{message}; recording its manual hold also failed: {recording}")
            }
        })
    }
}

pub(super) fn unix_now() -> Result<f64, ActionError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

fn admit_after_lookup_miss(
    db: &std::path::Path,
    context: ActionContext,
    prompt: &str,
) -> Result<StoredIngress, ActionError> {
    let event_id = context.discord_message_id.map(id_i64).transpose()?;
    let admission = admit(
        db,
        &NewIngress {
            ingress_id: format!("action:{}", Uuid::new_v4()),
            kind: IngressKind::Action,
            event_id,
            application_id: None,
            channel_id: id_i64(context.channel_id)?,
            owner_user_id: id_i64(context.user_id)?,
            source_message_id: event_id,
            payload: json!({"command":"new","prompt":prompt,"context":{
                "channel_id":context.channel_id,"user_id":context.user_id,
                "discord_message_id":context.discord_message_id,
                "auto_queue_when_busy":context.auto_queue_when_busy
            }}),
            target_thread_id: None,
            canonical_owner: None,
            now: unix_now()?,
        },
    )?;
    let record = admission.record.ok_or_else(|| {
        ActionError::Invalid(
            "this Discord request was already processed; no new thread was created".into(),
        )
    })?;
    validate_canonical_record(&record, context, prompt)?;
    Ok(record)
}

fn validate_canonical_record(
    record: &StoredIngress,
    context: ActionContext,
    prompt: &str,
) -> Result<(), ActionError> {
    if record.channel_id != id_i64(context.channel_id)?
        || record.owner_user_id != id_i64(context.user_id)?
        || record.event_id != context.discord_message_id.map(id_i64).transpose()?
        || cdr_store::ingress::new_command_prompt(record) != Some(prompt)
    {
        return Err(ActionError::Invalid(
            "this Discord request has a different original channel, user, command, or prompt"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
