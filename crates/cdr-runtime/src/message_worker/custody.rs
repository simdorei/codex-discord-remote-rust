use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cdr_store::StoreError;
use cdr_store::ingress::{IngressKind, NewIngress, begin_execution, confirm, hold};
use serde_json::json;

use super::classification::CandidateParts;

pub(super) fn request(parts: &CandidateParts, now: f64) -> Result<NewIngress, StoreError> {
    let plan = match &parts.frozen_plan {
        Ok(plan) => serde_json::to_value(plan)?,
        Err(error) => json!({"Error":error.to_string()}),
    };
    let attachments = parts.message.attachments.iter().map(|attachment| json!({
        "id":attachment.id.get(),"filename":attachment.filename,"size":attachment.size,
        "content_type":attachment.content_type,"artifact_status":"metadata_only_requires_reupload_if_unavailable"
    })).collect::<Vec<_>>();
    Ok(NewIngress {
        ingress_id: format!("message:{}", parts.persisted_id),
        kind: IngressKind::Message,
        event_id: Some(parts.persisted_id),
        application_id: None,
        channel_id: i64::try_from(parts.channel_id)
            .map_err(|_| StoreError::Integrity("invalid message channel".into()))?,
        owner_user_id: i64::try_from(parts.user_id)
            .map_err(|_| StoreError::Integrity("invalid message owner".into()))?,
        source_message_id: Some(parts.persisted_id),
        payload: json!({
            "version":1,"content":parts.message.content,"plan":plan,"attachments":attachments,
            "processing_mode":"normal","author_is_bot":parts.message.author.bot,
            "routing":{"mirrored_target":parts.routing_target,"selected_target":"resolved_before_processing"},
            "new_origin":parts.new_origin,"new_prompt_mention_arm":parts.new_prompt_mention_arm,
            "settings_binding":parts.settings_binding,"lifecycle_binding":parts.lifecycle_binding
        }),
        target_thread_id: parts
            .lifecycle_binding
            .as_ref()
            .map(|binding| binding.target.clone())
            .or_else(|| {
                parts
                    .settings_binding
                    .as_ref()
                    .map(|binding| binding.target.clone())
            })
            .or_else(|| parts.routing_target.clone()),
        canonical_owner: None,
        now,
    })
}

pub(super) struct MessageCustody {
    database: PathBuf,
    key: String,
    started: bool,
    finished: bool,
}

impl MessageCustody {
    pub(super) fn new(database: PathBuf, key: String) -> Self {
        Self {
            database,
            key,
            started: false,
            finished: false,
        }
    }

    pub(super) fn begin(&mut self, target: Option<&str>) -> Result<(), StoreError> {
        if !begin_execution(&self.database, &self.key, "processing", target, now()?)? {
            return Err(StoreError::Integrity(
                "message custody is no longer executable".into(),
            ));
        }
        self.started = true;
        Ok(())
    }

    pub(super) fn finish(&mut self) -> Result<(), StoreError> {
        confirm(&self.database, &self.key, now()?)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for MessageCustody {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let result = now().and_then(|now| {
            hold(
                &self.database,
                &self.key,
                "message processing ended before a durable handoff or confirmed response",
                !self.started,
                now,
            )
        });
        if let Err(error) = result {
            // The original row remains saved for startup recovery even if the
            // current failure cannot publish a hold. No payload is logged.
            eprintln!(
                "message_custody_hold_error request_id={} error={error}",
                self.key
            );
        }
    }
}

pub(super) fn now() -> Result<f64, StoreError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}
