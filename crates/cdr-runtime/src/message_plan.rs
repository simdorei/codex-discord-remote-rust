use std::collections::BTreeSet;

use thiserror::Error;

use crate::command_plan::CommandAction;
use crate::prefix_plan::{PrefixAction, PrefixPlanError, SkillPromptKind, plan_prefix};

const ATTACHMENT_PROMPT: &str = "Please inspect the attached Discord file(s).";

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "these independent facts are the input boundary for the pure Discord message gate"
)]
pub struct IncomingMessage<'a> {
    pub content: &'a str,
    pub message_content_enabled: bool,
    pub channel_allowed: bool,
    pub user_allowed: bool,
    pub author_is_bot: bool,
    pub author_is_self: bool,
    pub author_mentions_bridge: bool,
    pub has_attachments: bool,
    pub mirrored_target: bool,
    pub mentioned_user_ids: BTreeSet<u64>,
    pub required_plain_ask_user_ids: BTreeSet<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub enum MessagePlan {
    Ignore(&'static str),
    Respond(String),
    Execute(CommandAction),
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MessagePlanError {
    #[error(transparent)]
    Prefix(#[from] PrefixPlanError),
    #[error("prefix command is parsed but not implemented yet: !{0}")]
    UnsupportedPrefix(&'static str),
}

pub fn plan_message(input: &IncomingMessage<'_>) -> Result<MessagePlan, MessagePlanError> {
    if !input.message_content_enabled {
        return Ok(MessagePlan::Ignore("message_content_disabled"));
    }
    if !input.channel_allowed {
        return Ok(MessagePlan::Ignore("channel_not_allowed"));
    }
    if !input.user_allowed {
        return Ok(MessagePlan::Ignore("user_not_allowed"));
    }
    if input.author_is_self {
        return Ok(MessagePlan::Ignore("self_authored"));
    }
    if input.author_is_bot && !input.author_mentions_bridge {
        return Ok(MessagePlan::Ignore("bot_author_without_bridge_mention"));
    }

    let content = input.content.trim();
    if let Some(command) = content.strip_prefix('!') {
        return Ok(MessagePlan::Execute(prefix_to_command(plan_prefix(
            command,
        )?)?));
    }

    let mut prompt = content.to_owned();
    if !input.mirrored_target && !input.required_plain_ask_user_ids.is_empty() {
        let matched = input
            .mentioned_user_ids
            .iter()
            .any(|id| input.required_plain_ask_user_ids.contains(id));
        if !matched {
            return Ok(MessagePlan::Ignore("required_mention_missing"));
        }
        prompt = strip_required_mentions(&prompt, &input.required_plain_ask_user_ids);
        if prompt.is_empty() && !input.has_attachments {
            return Ok(MessagePlan::Respond(
                "Add a prompt after the mention.".into(),
            ));
        }
    }
    if prompt.is_empty() {
        if input.has_attachments {
            prompt = ATTACHMENT_PROMPT.into();
        } else {
            return Ok(MessagePlan::Ignore("empty_content"));
        }
    }
    Ok(MessagePlan::Execute(CommandAction::Ask { prompt }))
}

fn prefix_to_command(action: PrefixAction) -> Result<CommandAction, MessagePlanError> {
    Ok(match action {
        PrefixAction::Help => CommandAction::Help,
        PrefixAction::List { limit } => CommandAction::List {
            limit: if limit == 0 { 10 } else { limit },
        },
        PrefixAction::ArchivedList { limit } => CommandAction::ArchivedList { limit },
        PrefixAction::Use { reference } => CommandAction::Use { reference },
        PrefixAction::Status { reference } => CommandAction::Status { reference },
        PrefixAction::Settings {
            reference,
            model,
            effort,
            speed,
        } => CommandAction::Settings {
            reference,
            model,
            effort,
            speed,
        },
        PrefixAction::AutoReserve { reference, enabled } => {
            CommandAction::AutoReserve { reference, enabled }
        }
        PrefixAction::Doctor | PrefixAction::DiscoverCodex => CommandAction::Doctor,
        PrefixAction::Where => CommandAction::Where,
        PrefixAction::Context {
            all_threads,
            refresh,
            limit,
        } => CommandAction::Context {
            all_threads,
            refresh,
            limit,
        },
        PrefixAction::Usage { days } => CommandAction::Usage { days },
        PrefixAction::Runners => CommandAction::Runners,
        PrefixAction::SavedRequest { request_id } => CommandAction::SavedRequest { request_id },
        PrefixAction::Retract { reference } => CommandAction::Retract { reference },
        PrefixAction::BridgeSync { limit } => CommandAction::BridgeSync {
            limit: limit.map(i64::from),
        },
        PrefixAction::QaButtons => CommandAction::QaButtons,
        PrefixAction::New { prompt } => CommandAction::New { prompt },
        PrefixAction::Open { reference, abort } => CommandAction::Open { reference, abort },
        PrefixAction::Stop { reference } => CommandAction::Stop { reference },
        PrefixAction::SettingsOptions { reference, field } => {
            CommandAction::SettingsOptions { reference, field }
        }
        PrefixAction::RestartCodex => CommandAction::RestartCodex,
        PrefixAction::Archive { reference } => CommandAction::Archive { reference },
        PrefixAction::DeleteArchivePreview { reference } => {
            CommandAction::DeleteArchivePreview { reference }
        }
        PrefixAction::DeleteArchiveConfirm { reference } => {
            CommandAction::DeleteArchiveConfirm { reference }
        }
        PrefixAction::Resume { reference } => CommandAction::Resume { reference },
        PrefixAction::Identity => CommandAction::Identity,
        PrefixAction::Resources => CommandAction::Resources,
        PrefixAction::MirrorSync => CommandAction::BridgeSync { limit: None },
        PrefixAction::MirrorList { limit } => CommandAction::MirrorInspect { limit, list: true },
        PrefixAction::MirrorCheck { limit } => CommandAction::MirrorInspect { limit, list: false },
        PrefixAction::MirrorDetail { .. } => {
            return Err(MessagePlanError::UnsupportedPrefix("detail"));
        }
        PrefixAction::Approval => CommandAction::Approval,
        PrefixAction::Steer { prompt } => CommandAction::Steer { prompt },
        PrefixAction::HostReboot => CommandAction::HostReboot,
        PrefixAction::SkillPrompt {
            kind: SkillPromptKind::Interview,
            request,
        } => CommandAction::Interview { prompt: request },
        PrefixAction::SkillPrompt {
            kind: SkillPromptKind::Pro,
            request,
        } => CommandAction::Ask {
            prompt: format!("!pro {request}"),
        },
        PrefixAction::SkillPrompt {
            kind: SkillPromptKind::ArchiveUsed,
            request,
        } => CommandAction::Ask {
            prompt: format!("Use $archive-used with this threshold:\n\n{request}"),
        },
    })
}

fn strip_required_mentions(content: &str, ids: &BTreeSet<u64>) -> String {
    let mut result = content.to_owned();
    for id in ids {
        result = result.replace(&format!("<@{id}>"), "");
        result = result.replace(&format!("<@!{id}>"), "");
    }
    result.trim().to_owned()
}
