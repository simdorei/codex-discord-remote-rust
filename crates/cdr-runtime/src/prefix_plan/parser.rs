use super::grammar::{
    bounded, optional, plan_bridge, plan_context, plan_detail, plan_mirror, plan_qa, plan_reboot,
    plan_runners, plan_usage, required, skill,
};
use super::settings::plan_settings;
use super::{PrefixAction, PrefixPlanError, SkillPromptKind};

pub fn plan_prefix(command_line: &str) -> Result<PrefixAction, PrefixPlanError> {
    let (command, argument) = split_command(command_line);
    let arg = argument.trim();
    let action = match command.as_str() {
        "" | "help" | "start" => PrefixAction::Help,
        "list" => PrefixAction::List {
            limit: if arg.is_empty() {
                0
            } else {
                bounded(arg, 10, 1, 30)
            },
        },
        "archived_list" | "archive_list" => PrefixAction::ArchivedList {
            limit: bounded(arg, 10, 1, 50),
        },
        "use" => PrefixAction::Use {
            reference: required(arg, "Usage: !use <ref>")?,
        },
        "open" | "open_abort" => PrefixAction::Open {
            reference: required(arg, &format!("Usage: !{command} <ref>"))?,
            abort: command == "open_abort",
        },
        "status" => PrefixAction::Status {
            reference: optional(arg),
        },
        "stop" => PrefixAction::Stop {
            reference: optional(arg),
        },
        "settings" | "setting" => plan_settings(arg)?,
        "discover_codex" => PrefixAction::DiscoverCodex,
        "restart_codex" => PrefixAction::RestartCodex,
        "archive" => PrefixAction::Archive {
            reference: optional(arg),
        },
        "delete_archive" => PrefixAction::DeleteArchivePreview {
            reference: required(arg, "Usage: !delete_archive <ref>")?,
        },
        "confirm_delete_archive" => PrefixAction::DeleteArchiveConfirm {
            reference: required(arg, "Usage: !confirm_delete_archive <ref>")?,
        },
        "doctor" => PrefixAction::Doctor,
        "resume" => PrefixAction::Resume {
            reference: optional(arg),
        },
        "chatid" | "whoami" => PrefixAction::Identity,
        "where" | "map" => PrefixAction::Where,
        "context" | "ctx" => plan_context(arg)?,
        "usage" | "quota" | "limit" => plan_usage(arg)?,
        "runners" | "queues" => plan_runners(arg)?,
        "resources" | "system" => PrefixAction::Resources,
        "retract" | "unqueue" => PrefixAction::Retract {
            reference: optional(arg),
        },
        "bridge_sync" | "resync" | "sync" | "bridge" => plan_bridge(&command, arg)?,
        "mirror" => plan_mirror(arg)?,
        "detail" => plan_detail(arg)?,
        "approval" | "approve" => PrefixAction::Approval,
        "new" => PrefixAction::New {
            prompt: arg.to_owned(),
        },
        "steer" => PrefixAction::Steer {
            prompt: required(arg, "Usage: !steer <prompt>")?,
        },
        "qa" => plan_qa(arg)?,
        "pro" => skill(SkillPromptKind::Pro, &command, arg, "request")?,
        "interview" | "deep_interview" | "deep-interview" => {
            skill(SkillPromptKind::Interview, &command, arg, "request")?
        }
        "archive-used" => skill(SkillPromptKind::ArchiveUsed, &command, arg, "threshold")?,
        "reset_pc" | "reboot_pc" | "reset_computer" => plan_reboot(arg)?,
        unknown => return Err(PrefixPlanError::Unknown(unknown.into())),
    };
    Ok(action)
}

fn split_command(command_line: &str) -> (String, &str) {
    let trimmed = command_line.trim_start();
    let (command, arg) = trimmed
        .split_once(char::is_whitespace)
        .unwrap_or((trimmed, ""));
    (command.trim().to_lowercase(), arg)
}
