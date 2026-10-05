//! Internal consistency of saved facts only. Never infer a live route or effect.
use serde_json::{Value, json};

pub(super) fn inspect(metadata: &Value, encoded: Option<&str>) -> Value {
    let payload = encoded
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .filter(Value::is_object);
    let binding = payload
        .as_ref()
        .and_then(|value| value.get("lifecycle_binding"));
    let command = payload
        .as_ref()
        .and_then(|value| value.pointer("/plan/Execute"));
    let control = command.and_then(control);
    let route = binding
        .and_then(|value| value.get("route"))
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "Explicit" | "Mapped" | "Selected"));
    let shape = binding
        .and_then(Value::as_object)
        .is_some_and(|value| value.len() == 3)
        && binding
            .and_then(|value| value.get("target"))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty() && value.len() <= 512);
    let target_matches =
        shape && binding.and_then(|value| value.get("target")) == metadata.get("thread_id");
    let command_matches =
        control.is_some() && command == binding.and_then(|value| value.get("command"));
    let route_shape = matches!(
        (route, control),
        (Some("Explicit"), Some((_, true))) | (Some("Mapped" | "Selected"), Some((_, false)))
    );
    let source_consistent = source_consistent(metadata)
        && payload
            .as_ref()
            .and_then(|value| value.get("version"))
            .and_then(Value::as_i64)
            == Some(1);
    let consistent = source_consistent && target_matches && command_matches && route_shape;
    json!({
        "diagnostic_only":true,"replay_authorized":false,
        "live_route_verified":false,"effect_verified":false,
        "payload_status":if encoded.is_none(){"unavailable"}else if payload.is_some(){"valid"}else{"invalid"},
        "binding_presence":match binding {
            None if payload.is_none()=>"unknown",
            None=>"missing",Some(Value::Null)=>"null",Some(Value::Object(_))=>"object",Some(_)=>"invalid",
        },
        "declared_control":control.map_or("unknown_or_other",|(kind,_)|kind),
        "route":route,"stored_source_envelope_consistent":source_consistent,
        "stored_target_matches":target_matches,"stored_command_matches":command_matches,
        "stored_custody_consistent":consistent,
        "disposition":if consistent {"preserve_reconcile_original_effect"}
            else {"preserve_require_fresh_bound_intent"},
        "existing_obligations_preserved":true
    })
}

fn control(value: &Value) -> Option<(&'static str, bool)> {
    let command = value.as_object().filter(|value| value.len() == 1)?;
    let kind = ["Stop", "Archive"]
        .into_iter()
        .find(|kind| command.contains_key(*kind))?;
    let fields = command[kind].as_object().filter(|value| value.len() == 1)?;
    let reference = fields.get("reference")?;
    if reference.is_null() {
        return Some((kind, false));
    }
    reference
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(|_| (kind, true))
}

fn source_consistent(metadata: &Value) -> bool {
    let event = metadata
        .get("event_id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    metadata.get("kind").and_then(Value::as_str) == Some("message")
        && event.is_some_and(|event| {
            metadata.get("source_message_id").and_then(Value::as_i64) == Some(event)
                && metadata.get("ingress_id").and_then(Value::as_str)
                    == Some(format!("message:{event}").as_str())
        })
        && metadata
            .get("channel_id")
            .and_then(Value::as_i64)
            .is_some_and(|value| value > 0)
        && metadata
            .get("owner_user_id")
            .and_then(Value::as_i64)
            .is_some_and(|value| value > 0)
        && metadata.get("owner_kind").is_some_and(Value::is_null)
        && metadata.get("owner_id").is_some_and(Value::is_null)
}
