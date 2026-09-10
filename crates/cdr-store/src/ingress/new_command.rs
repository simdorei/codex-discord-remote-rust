use super::{IngressKind, StoredIngress};

/// Read the original new-command prompt without rewriting its ingress envelope.
/// Each transport has its own persisted representation; unknown versions fail closed.
pub fn new_command_prompt(ingress: &StoredIngress) -> Option<&str> {
    let payload = &ingress.payload;
    match ingress.kind {
        IngressKind::Action if payload.get("command")?.as_str()? == "new" => {
            payload.get("prompt")?.as_str()
        }
        IngressKind::Message if payload.get("version")?.as_u64()? == 1 => {
            payload.pointer("/plan/Execute/New/prompt")?.as_str()
        }
        IngressKind::Interaction if payload.get("version")?.as_u64()? == 1 => {
            let invocation = payload.pointer("/work/Slash")?;
            if invocation.get("name")?.as_str()? != "new" {
                return None;
            }
            invocation
                .pointer("/values/prompt/String")?
                .as_str()
                .map(str::trim)
        }
        _ => None,
    }
}
