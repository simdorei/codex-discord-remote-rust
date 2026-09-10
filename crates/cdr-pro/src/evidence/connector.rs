use std::path::Path;

use serde_json::{Map, Value};

use super::common::{read_json_object, receipt_key};
use super::connector_transcript::{
    CONNECTOR_PROBE_SHA256, CONNECTOR_PROTOCOL, read_connector_transcript, valid_connector_evidence,
};
use super::{Result, connector_unavailable};

pub fn require_connector_verified(
    session_id: &str,
    turn_id: &str,
    plugin_data: &Path,
    codex_home: &Path,
    plugin_root: &Path,
) -> Result<()> {
    let receipt_path = plugin_data
        .join("pro-connector-evidence")
        .join(format!("{}.json", receipt_key(session_id, turn_id)));
    let transcript = read_connector_transcript(session_id, turn_id, codex_home, plugin_root);
    let evidence = if receipt_path.exists() {
        let receipt = read_receipt(session_id, turn_id, &receipt_path).ok_or_else(|| {
            connector_unavailable(
                "Exact-turn connector receipt exists but is invalid or unreadable.",
            )
        })?;
        if transcript.trusted_attempt_seen {
            transcript.evidence
        } else {
            Some(receipt)
        }
    } else {
        transcript.evidence
    };
    let evidence = evidence.ok_or_else(|| {
        connector_unavailable("Pro turn completed without exact-turn connector control evidence.")
    })?;
    if evidence.get("status").and_then(Value::as_str) != Some("verified") {
        return Err(connector_unavailable(
            "Chrome connector control was not verified.",
        ));
    }
    if !valid_connector_evidence(&evidence) {
        return Err(connector_unavailable(
            "Connector, Chat mode, or Pro mode evidence did not match the contract.",
        ));
    }
    Ok(())
}

fn read_receipt(session_id: &str, turn_id: &str, path: &Path) -> Option<Map<String, Value>> {
    if session_id.is_empty() || turn_id.is_empty() {
        return None;
    }
    let values = read_json_object(path)?;
    let valid = values.get("protocol").and_then(Value::as_str) == Some(CONNECTOR_PROTOCOL)
        && values.get("session_id").and_then(Value::as_str) == Some(session_id)
        && values.get("turn_id").and_then(Value::as_str) == Some(turn_id)
        && values.get("browser_type").and_then(Value::as_str) == Some("chrome")
        && values.get("probe_sha256").and_then(Value::as_str) == Some(CONNECTOR_PROBE_SHA256)
        && valid_connector_evidence(&values);
    valid.then_some(values)
}
