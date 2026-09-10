use super::{MirrorDetailMode, PrefixAction, PrefixPlanError, SkillPromptKind};

pub(super) fn optional(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn required(value: &str, usage: &str) -> Result<String, PrefixPlanError> {
    optional(value).ok_or_else(|| PrefixPlanError::Usage(usage.into()))
}

pub(super) fn bounded(raw: &str, default: u32, minimum: u32, maximum: u32) -> u32 {
    raw.parse::<i64>()
        .ok()
        .and_then(|value| u32::try_from(value.clamp(i64::from(minimum), i64::from(maximum))).ok())
        .unwrap_or(default)
}

fn required_bounded(
    raw: &str,
    default: u32,
    maximum: u32,
    usage: &str,
) -> Result<u32, PrefixPlanError> {
    if raw.is_empty() {
        return Ok(default);
    }
    raw.parse::<i64>()
        .ok()
        .and_then(|value| u32::try_from(value.clamp(1, i64::from(maximum))).ok())
        .ok_or_else(|| PrefixPlanError::Usage(usage.into()))
}

pub(super) fn plan_context(arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    let words = arg
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if matches!(
        words.first().map(String::as_str),
        Some("refresh" | "recent")
    ) {
        if words.len() > 2 {
            return Err(PrefixPlanError::Usage(
                "Usage: !context [all | refresh [limit]]".into(),
            ));
        }
        let limit = required_bounded(
            words.get(1).map_or("", String::as_str),
            10,
            30,
            "Usage: !context refresh [integer limit]",
        )?;
        return Ok(PrefixAction::Context {
            all_threads: false,
            refresh: true,
            limit,
        });
    }
    let all_threads = matches!(arg.trim().to_lowercase().as_str(), "all" | "*");
    if !words.is_empty() && !all_threads {
        return Err(PrefixPlanError::Usage(
            "Usage: !context [all | refresh [limit]]".into(),
        ));
    }
    Ok(PrefixAction::Context {
        all_threads,
        refresh: false,
        limit: if all_threads { 20 } else { 10 },
    })
}

pub(super) fn plan_usage(arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    Ok(PrefixAction::Usage {
        days: required_bounded(arg, 7, 30, "Usage: !usage [days]")?,
    })
}

pub(super) fn plan_runners(arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    if arg.is_empty() {
        return Ok(PrefixAction::Runners);
    }
    if arg.chars().any(char::is_whitespace) {
        return Err(PrefixPlanError::Usage(
            "Usage: !runners [request_id]".into(),
        ));
    }
    Ok(PrefixAction::SavedRequest {
        request_id: arg.to_owned(),
    })
}

pub(super) fn skill(
    kind: SkillPromptKind,
    command: &str,
    arg: &str,
    name: &str,
) -> Result<PrefixAction, PrefixPlanError> {
    Ok(PrefixAction::SkillPrompt {
        kind,
        request: required(arg, &format!("Usage: !{command} <{name}>"))?,
    })
}

pub(super) fn plan_reboot(arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    if arg.eq_ignore_ascii_case("confirm") {
        Ok(PrefixAction::HostReboot)
    } else {
        Err(PrefixPlanError::Usage("Usage: !reset_pc confirm".into()))
    }
}

pub(super) fn plan_qa(arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    if arg.is_empty() || matches!(arg.to_lowercase().as_str(), "button" | "buttons") {
        Ok(PrefixAction::QaButtons)
    } else {
        Err(PrefixPlanError::Usage("Usage: !qa buttons".into()))
    }
}

pub(super) fn plan_detail(arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    let mode = match arg.to_lowercase().as_str() {
        "" => None,
        "send" => Some(MirrorDetailMode::Send),
        "all" => Some(MirrorDetailMode::All),
        _ => {
            return Err(PrefixPlanError::Usage(
                "Usage: !detail | !detail send | !detail all".into(),
            ));
        }
    };
    Ok(PrefixAction::MirrorDetail { mode })
}

pub(super) fn plan_bridge(command: &str, arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    let raw = if command == "bridge" {
        let (subcommand, tail) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
        if !subcommand.is_empty() && !subcommand.eq_ignore_ascii_case("sync") {
            return Err(PrefixPlanError::Usage("Usage: !bridge sync [limit]".into()));
        }
        tail.trim()
    } else {
        arg
    };
    let limit = if raw.is_empty() {
        None
    } else {
        Some(required_bounded(
            raw,
            1,
            100,
            "Usage: !bridge sync [limit]",
        )?)
    };
    Ok(PrefixAction::BridgeSync { limit })
}

pub(super) fn plan_mirror(arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    let (subcommand, raw_limit) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
    match subcommand.to_lowercase().as_str() {
        "" | "sync" if raw_limit.trim().is_empty() => Ok(PrefixAction::MirrorSync),
        "list" => Ok(PrefixAction::MirrorList {
            limit: mirror_limit(raw_limit, "list")?,
        }),
        "check" | "doctor" => Ok(PrefixAction::MirrorCheck {
            limit: mirror_limit(raw_limit, "check")?,
        }),
        _ => Err(PrefixPlanError::Usage(
            "Usage: !mirror sync | !mirror list [limit] | !mirror check [limit]".into(),
        )),
    }
}

fn mirror_limit(raw: &str, subcommand: &str) -> Result<Option<u32>, PrefixPlanError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    required_bounded(raw, 1, 100, &format!("Usage: !mirror {subcommand} [limit]")).map(Some)
}
