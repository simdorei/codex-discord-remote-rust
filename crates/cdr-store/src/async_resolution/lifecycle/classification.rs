//! Positive recognition of stored non-control shapes, not execution authority.
//! Unknown variants or incomplete discriminators remain held. Keep these shapes
//! aligned with the runtime serializers; do not reparse raw user text as a plan.
use serde_json::Value;

#[cfg(test)]
#[path = "publication_classification_tests.rs"]
mod publication_tests;

#[cfg(test)]
#[path = "abandonment_classification_tests.rs"]
mod abandonment_tests;

pub(super) fn ordinary(payload: &Value) -> bool {
    if payload.get("version").and_then(Value::as_u64) != Some(1) {
        return false;
    }
    let mut recognized = false;
    for key in ["plan", "lifecycle_binding", "work", "command"] {
        let Some(value) = payload.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        let valid = match key {
            "plan" => single(value).is_some_and(|(kind, body)| match kind {
                "Execute" => command(body),
                "Respond" | "Error" | "Ignore" => body.is_string(),
                _ => false,
            }),
            "lifecycle_binding" => {
                value.is_object()
                    && strings(value, &["target"])
                    && matches!(
                        value["route"].as_str(),
                        Some("Mapped" | "Explicit" | "Selected")
                    )
                    && command(&value["command"])
            }
            "work" => work(value),
            "command" => value.as_str().is_some_and(slash_name),
            _ => false,
        };
        if !valid {
            return false;
        }
        recognized = true;
    }
    recognized
}

fn single(value: &Value) -> Option<(&str, &Value)> {
    let fields = value.as_object()?;
    if fields.len() != 1 {
        return None;
    }
    fields
        .iter()
        .next()
        .map(|(key, value)| (key.as_str(), value))
}

fn strings(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .all(|key| value.get(*key).is_some_and(Value::is_string))
}

fn optional_strings(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .all(|key| value.get(*key).is_none_or(|v| v.is_null() || v.is_string()))
}

fn u32_field(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_u64)
        .is_some_and(|n| u32::try_from(n).is_ok())
}

fn optional_limit(value: &Value, signed: bool) -> bool {
    value.get("limit").is_none_or(|v| {
        v.is_null()
            || if signed {
                v.as_i64().is_some()
            } else {
                u32_field(value, "limit")
            }
    })
}

fn command(value: &Value) -> bool {
    if let Some(name) = value.as_str() {
        return matches!(
            name,
            "Help"
                | "Where"
                | "Doctor"
                | "Runners"
                | "MirrorCheck"
                | "QaButtons"
                | "RestartCodex"
                | "ForceRestartCodex"
                | "Identity"
                | "Resources"
                | "Approval"
                | "HostReboot"
        );
    }
    let Some((name, fields)) = single(value).filter(|(_, body)| body.is_object()) else {
        return false;
    };
    match name {
        "Ask" | "New" | "Interview" | "Steer" => strings(fields, &["prompt"]),
        "List" | "ArchivedList" => u32_field(fields, "limit"),
        "Usage" => u32_field(fields, "days"),
        "Use" | "DeleteArchivePreview" | "DeleteArchiveConfirm" => strings(fields, &["reference"]),
        "SavedRequest" => strings(fields, &["request_id"]),
        "DiscardRequest" => {
            fields.as_object().is_some_and(|v| v.len() == 1) && canonical_job_id(&fields["job_id"])
        }
        "Status" | "Retract" | "Recover" | "Repair" | "Resume" => {
            optional_strings(fields, &["reference"])
        }
        "Settings" => optional_strings(fields, &["reference", "model", "effort", "speed"]),
        "SettingsOptions" => optional_strings(fields, &["reference", "field"]),
        "AutoReserve" => optional_strings(fields, &["reference"]) && fields["enabled"].is_boolean(),
        "Context" => {
            fields["all_threads"].is_boolean()
                && fields["refresh"].is_boolean()
                && u32_field(fields, "limit")
        }
        "MirrorInspect" => optional_limit(fields, false) && fields["list"].is_boolean(),
        "BridgeSync" => optional_limit(fields, true),
        "Open" => strings(fields, &["reference"]) && fields["abort"].is_boolean(),
        _ => false,
    }
}

fn slash_name(name: &str) -> bool {
    matches!(
        name,
        "help"
            | "list"
            | "archived_list"
            | "use"
            | "status"
            | "settings"
            | "where"
            | "context"
            | "usage"
            | "new"
            | "ask"
            | "interview"
            | "doctor"
            | "approval"
            | "runners"
            | "retract"
            | "mirror_check"
            | "bridge_sync"
            | "qa_buttons"
    )
}

fn work(value: &Value) -> bool {
    let Some((kind, fields)) = single(value).filter(|(_, body)| body.is_object()) else {
        return false;
    };
    match kind {
        "Slash" => {
            fields["name"].as_str().is_some_and(slash_name)
                && fields["values"].as_object().is_some_and(|values| {
                    values.values().all(|v| {
                        single(v).is_some_and(|(kind, body)| match kind {
                            "Boolean" => body.is_boolean(),
                            "Integer" => body.as_i64().is_some(),
                            "String" => body.is_string(),
                            _ => false,
                        })
                    })
                })
        }
        "Autocomplete" => {
            strings(fields, &["command_name", "option_name", "current"])
                && optional_strings(fields, &["selected_model"])
        }
        "Component" => component(fields),
        _ => false,
    }
}

fn component(value: &Value) -> bool {
    let Some((kind, fields)) = single(value).filter(|(_, body)| body.is_object()) else {
        return false;
    };
    match kind {
        "AsyncChoice" => {
            strings(fields, &["question_id"]) && fields["option"].as_u64().is_some_and(|n| n < 25)
        }
        "RecoveryPublicationDecision" => {
            fields.as_object().is_some_and(|v| v.len() == 3)
                && fields["proposal_id"].as_str().is_some_and(|id| {
                    id.len() == 32
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                })
                && fields["revision"].as_i64().is_some_and(|n| n > 0)
                && matches!(
                    fields["decision"].as_str(),
                    Some("ApproveExact" | "KeepHeld")
                )
        }
        "RecoveryAbandonDecision" => {
            fields.as_object().is_some_and(|v| v.len() == 3)
                && fields["proposal_id"].as_str().is_some_and(|id| {
                    id.len() == 32
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                })
                && fields["revision"].as_i64().is_some_and(|n| n > 0)
                && matches!(
                    fields["decision"].as_str(),
                    Some("AbandonOnly" | "KeepHeld")
                )
        }
        "Busy" => {
            strings(fields, &["choice_id"])
                && matches!(
                    fields["action"].as_str(),
                    Some("Steer" | "Queue" | "Ignore")
                )
        }
        "Input" => strings(fields, &["thread_id", "value"]),
        "BoundInput" => strings(
            fields,
            &["thread_fingerprint", "request_fingerprint", "value"],
        ),
        "Approval" | "BoundApproval" => {
            let identity = if kind == "Approval" {
                strings(fields, &["thread_id"])
            } else {
                strings(fields, &["thread_fingerprint", "request_fingerprint"])
            };
            identity
                && matches!(
                    fields["answer"].as_str(),
                    Some("Approve" | "ApproveSession" | "Reject" | "Cancel")
                )
        }
        _ => false,
    }
}

fn canonical_job_id(value: &Value) -> bool {
    value.as_str().is_some_and(|id| {
        id.len() == 36
            && id.bytes().enumerate().all(|(index, b)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
                }
            })
    })
}
