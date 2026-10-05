use serde::Serialize;
use twilight_model::channel::message::component::{ActionRow, Button, ButtonStyle, Component};

use super::{ComponentError, ComponentId, bounded, valid_fingerprint};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PublicationDecision {
    ApproveExact,
    KeepHeld,
}

pub(super) fn parse(id: &str, revision: &str, choice: &str) -> Option<ComponentId> {
    let number = revision.parse::<i64>().ok()?;
    if !valid_fingerprint(id, 32) || number <= 0 || number.to_string() != revision {
        return None;
    }
    let decision = match choice {
        "a" => PublicationDecision::ApproveExact,
        "h" => PublicationDecision::KeepHeld,
        _ => return None,
    };
    Some(ComponentId::RecoveryPublicationDecision {
        proposal_id: id.into(),
        revision: number,
        decision,
    })
}

/// These buttons record intent only. They are not app-server approvals.
pub fn publication_decision_rows(
    id: &str,
    revision: i64,
) -> Result<Vec<Component>, ComponentError> {
    if !valid_fingerprint(id, 32) || revision <= 0 {
        return Err(ComponentError::Invalid);
    }
    let buttons = [
        ("a", "Approve exact recovery intent", ButtonStyle::Primary),
        ("h", "Keep recovery held", ButtonStyle::Secondary),
    ]
    .into_iter()
    .map(|(choice, label, style)| {
        Ok(Component::Button(Button {
            id: None,
            custom_id: Some(bounded(format!("codex_pub:v1:{id}:{revision}:{choice}"))?),
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
