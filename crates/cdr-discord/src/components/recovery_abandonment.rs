use super::{ComponentError, ComponentId, bounded, valid_fingerprint};
use serde::Serialize;
use twilight_model::channel::message::component::{ActionRow, Button, ButtonStyle, Component};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AbandonDecision {
    AbandonOnly,
    KeepHeld,
}

pub(super) fn parse(id: &str, revision: &str, choice: &str) -> Option<ComponentId> {
    let number = revision.parse::<i64>().ok()?;
    if !valid_fingerprint(id, 32) || number <= 0 || number.to_string() != revision {
        return None;
    }
    let decision = match choice {
        "a" => AbandonDecision::AbandonOnly,
        "h" => AbandonDecision::KeepHeld,
        _ => return None,
    };
    Some(ComponentId::RecoveryAbandonDecision {
        proposal_id: id.into(),
        revision: number,
        decision,
    })
}

/// Explicit disposition only: neither button releases a thread or grants RPC authority.
pub fn abandonment_decision_rows(
    id: &str,
    revision: i64,
) -> Result<Vec<Component>, ComponentError> {
    if !valid_fingerprint(id, 32) || revision <= 0 {
        return Err(ComponentError::Invalid);
    }
    let buttons = [
        ("a", "Abandon saved request only", ButtonStyle::Danger),
        ("h", "Keep held", ButtonStyle::Secondary),
    ]
    .into_iter()
    .map(|(choice, label, style)| {
        Ok(Component::Button(Button {
            id: None,
            custom_id: Some(bounded(format!(
                "codex_discard:v1:{id}:{revision}:{choice}"
            ))?),
            disabled: false,
            emoji: None,
            label: Some(label.into()),
            style,
            url: None,
            sku_id: None,
        }))
    })
    .collect::<Result<Vec<_>, ComponentError>>()?;
    Ok(vec![Component::ActionRow(ActionRow {
        id: None,
        components: buttons,
    })])
}
