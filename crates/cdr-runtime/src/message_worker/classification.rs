use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_store::mapping::{NewThreadOrigin, new_thread_origin};
use twilight_model::channel::Message;

use super::admission::MessageAdmissionError;
use crate::config::RuntimeConfig;
use crate::message_plan::{IncomingMessage, MessagePlan, MessagePlanError, plan_message};

#[path = "lifecycle_binding.rs"]
mod lifecycle_binding;

pub(crate) enum MessageClassification {
    Ignore(IgnoredMessage),
    Candidate(MessageCandidate),
}

pub(crate) struct IgnoredMessage {
    reason: &'static str,
    channel_id: u64,
    user_id: u64,
}

impl IgnoredMessage {
    pub(crate) fn into_log_parts(self) -> (&'static str, u64, u64) {
        (self.reason, self.channel_id, self.user_id)
    }
}

pub(crate) struct MessageCandidate {
    message: Box<Message>,
    database: PathBuf,
    persisted_id: i64,
    channel_id: u64,
    user_id: u64,
    frozen_plan: Result<MessagePlan, MessagePlanError>,
    routing_target: Option<String>,
    new_origin: Box<NewThreadOrigin>,
    new_prompt_mention_arm: Option<String>,
    settings_binding: Option<Box<crate::settings_binding::SettingsBinding>>,
    lifecycle_binding: Option<Box<crate::settings_binding::SettingsBinding>>,
}

pub(super) struct CandidateParts {
    pub message: Box<Message>,
    pub database: PathBuf,
    pub persisted_id: i64,
    pub channel_id: u64,
    pub user_id: u64,
    pub frozen_plan: Result<MessagePlan, MessagePlanError>,
    pub routing_target: Option<String>,
    pub new_origin: Box<NewThreadOrigin>,
    pub new_prompt_mention_arm: Option<String>,
    pub settings_binding: Option<Box<crate::settings_binding::SettingsBinding>>,
    pub lifecycle_binding: Option<Box<crate::settings_binding::SettingsBinding>>,
}

impl MessageCandidate {
    pub(crate) fn bind_settings(
        mut self,
        resolver: &crate::settings_binding::SettingsTargetResolver,
    ) -> Result<Self, MessageAdmissionError> {
        self.lifecycle_binding = lifecycle_binding::bind(
            &mut self.frozen_plan,
            self.routing_target.as_deref(),
            resolver,
            self.channel_id,
        )?;
        if let Ok(MessagePlan::Execute(action)) = &self.frozen_plan {
            self.settings_binding = match resolver.bind(action, self.channel_id) {
                Ok(binding) => binding.map(Box::new),
                Err(error) if crate::settings_binding::is_request_rejection(&error) => {
                    // Preserve the raw message in durable custody, but freeze a
                    // non-executable response so history replay cannot retarget it.
                    self.frozen_plan = Ok(MessagePlan::Respond(format!("ERROR: {error}")));
                    return Ok(self);
                }
                Err(error) => {
                    return Err(cdr_store::StoreError::Integrity(error.to_string()).into());
                }
            };
            if let Some(binding) = &self.settings_binding {
                let same_route = match binding.route {
                    crate::settings_binding::SettingsRoute::Explicit => true,
                    crate::settings_binding::SettingsRoute::Mapped => {
                        self.routing_target.as_deref() == Some(binding.target.as_str())
                    }
                    crate::settings_binding::SettingsRoute::Selected => {
                        self.routing_target.is_none()
                    }
                };
                if !same_route {
                    self.settings_binding = None;
                    self.frozen_plan = Ok(MessagePlan::Respond("ERROR: settings mapping changed during message classification; no update was sent".into()));
                }
            }
        }
        Ok(self)
    }
    pub(crate) fn is_pending_reply_candidate(&self) -> bool {
        matches!(
            &self.frozen_plan,
            Ok(MessagePlan::Execute(
                crate::command_plan::CommandAction::Ask { prompt }
            )) if !cdr_pro::prompt::is_pro_command(prompt)
        )
    }

    pub(crate) fn is_stop_control(&self) -> bool {
        matches!(
            &self.frozen_plan,
            Ok(MessagePlan::Execute(
                crate::command_plan::CommandAction::Stop { .. }
            ))
        )
    }

    pub(super) fn into_admission_parts(self) -> CandidateParts {
        CandidateParts {
            message: self.message,
            database: self.database,
            persisted_id: self.persisted_id,
            channel_id: self.channel_id,
            user_id: self.user_id,
            frozen_plan: self.frozen_plan,
            routing_target: self.routing_target,
            new_origin: self.new_origin,
            new_prompt_mention_arm: self.new_prompt_mention_arm,
            settings_binding: self.settings_binding,
            lifecycle_binding: self.lifecycle_binding,
        }
    }
}

pub(crate) fn classify_gateway_message(
    message: Message,
    database: &Path,
    config: &RuntimeConfig,
    policy: &InteractionAccessPolicy,
    bot_user_id: Option<u64>,
) -> Result<MessageClassification, MessageAdmissionError> {
    classify_gateway_message_with(message, database, config, policy, bot_user_id, plan_message)
}

pub(super) fn classify_gateway_message_with<F>(
    message: Message,
    database: &Path,
    config: &RuntimeConfig,
    policy: &InteractionAccessPolicy,
    bot_user_id: Option<u64>,
    planner: F,
) -> Result<MessageClassification, MessageAdmissionError>
where
    F: FnOnce(&IncomingMessage<'_>) -> Result<MessagePlan, MessagePlanError>,
{
    let persisted_id =
        i64::try_from(message.id.get()).map_err(|_| MessageAdmissionError::IntegerRange)?;
    let channel_id = message.channel_id.get();
    let user_id = message.author.id.get();
    let stored_channel_id =
        i64::try_from(channel_id).map_err(|_| MessageAdmissionError::IntegerRange)?;
    let new_origin = new_thread_origin(database, stored_channel_id)?;
    let routing_target = new_origin.target.clone();
    let mirrored_target = routing_target.is_some();
    let mentioned_user_ids = message
        .mentions
        .iter()
        .map(|mention| mention.id.get())
        .collect::<BTreeSet<_>>();
    let new_prompt_mention_arm = if !mirrored_target
        && !message.author.bot
        && !message.content.trim_start().starts_with('!')
        && !config.plain_ask_mention_user_ids.is_empty()
        && mentioned_user_ids.is_disjoint(&config.plain_ask_mention_user_ids)
    {
        cdr_store::ingress::pending_new_prompt(
            database,
            stored_channel_id,
            i64::try_from(user_id).map_err(|_| MessageAdmissionError::IntegerRange)?,
            persisted_id,
        )?
    } else {
        None
    };
    let frozen_plan = planner(&IncomingMessage {
        content: &message.content,
        message_content_enabled: config.enable_message_content,
        channel_allowed: policy.allow_all_channels
            || policy.allowed_channel_ids.contains(&channel_id)
            || policy.mirrored_channel_ids.contains(&channel_id),
        user_allowed: policy.allowed_user_ids.is_empty()
            || policy.allowed_user_ids.contains(&user_id),
        author_is_bot: message.author.bot,
        author_is_self: bot_user_id == Some(user_id),
        author_mentions_bridge: bot_user_id.is_some_and(|id| mentioned_user_ids.contains(&id)),
        has_attachments: !message.attachments.is_empty(),
        mirrored_target: mirrored_target || new_prompt_mention_arm.is_some(),
        mentioned_user_ids,
        required_plain_ask_user_ids: config.plain_ask_mention_user_ids.clone(),
    });
    match frozen_plan {
        Ok(MessagePlan::Ignore(reason)) => Ok(MessageClassification::Ignore(IgnoredMessage {
            reason,
            channel_id,
            user_id,
        })),
        frozen_plan => Ok(MessageClassification::Candidate(MessageCandidate {
            message: Box::new(message),
            database: database.to_path_buf(),
            persisted_id,
            channel_id,
            user_id,
            frozen_plan,
            routing_target,
            new_origin: Box::new(new_origin),
            new_prompt_mention_arm,
            settings_binding: None,
            lifecycle_binding: None,
        })),
    }
}

#[cfg(test)]
#[path = "classification_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "classification_failure_tests.rs"]
mod failure_tests;
