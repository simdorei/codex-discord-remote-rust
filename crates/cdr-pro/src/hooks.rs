//! Evidence hooks share the runtime's canonical probes and receipt contracts.
use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::evidence::{self, common, connector_transcript};

mod claims;

#[derive(Clone, Copy)]
pub enum HookKind {
    Browser,
    Connector,
}

impl HookKind {
    const fn directory(self) -> &'static str {
        match self {
            Self::Browser => "browser-evidence",
            Self::Connector => "pro-connector-evidence",
        }
    }
    const fn probe(self) -> (&'static str, &'static str) {
        match self {
            Self::Browser => (
                "skills/ask-chatgpt-pro/scripts/browser_evidence_probe.mjs",
                evidence::BROWSER_PROBE_SHA256,
            ),
            Self::Connector => (
                "skills/ask-chatgpt-pro/scripts/pro_connector_control.mjs",
                evidence::CONNECTOR_PROBE_SHA256,
            ),
        }
    }
}

pub fn handle(
    kind: HookKind,
    payload: &Value,
    root: &Path,
    data: &Path,
) -> Result<Option<Value>, String> {
    if matches!(kind, HookKind::Browser) && payload["hook_event_name"] == "Stop" {
        return Ok(claims::check(payload, data));
    }
    if payload["hook_event_name"] != "PostToolUse" || !trusted(kind, payload, root)? {
        return Ok(None);
    }
    let (path, digest) = kind.probe();
    if !common::probe_integrity_ok(root, Path::new(path), digest) {
        return Err("evidence_hook_failed stage=probe_integrity_failed".into());
    }
    let session = identity(payload, "session_id")?;
    let turn = identity(payload, "turn_id")?;
    let candidates = common::embedded_objects(&payload["tool_response"]);
    let mut receipt = match kind {
        HookKind::Browser => {
            let Some(value) = candidates.into_iter().find(valid_browser) else {
                return Ok(None);
            };
            // Do not persist arbitrary tool output or diagnostic text.
            let mut receipt = Map::new();
            for field in [
                "protocol",
                "browser_type",
                "status",
                "can_report_unavailable",
            ] {
                receipt.insert(field.into(), value[field].clone());
            }
            receipt.insert(
                "tool_use_id".into(),
                json!(payload["tool_use_id"].as_str().unwrap_or("")),
            );
            receipt
        }
        HookKind::Connector => {
            let candidates: Vec<_> = candidates
                .into_iter()
                .filter(connector_transcript::valid_connector_evidence)
                .collect();
            if candidates.len() == 1 {
                let mut receipt = Map::new();
                for field in [
                    "protocol",
                    "browser_type",
                    "status",
                    "connector_name",
                    "connector_path",
                    "chat_mode",
                    "pro_mode",
                    "action",
                    "failed_stage",
                ] {
                    if let Some(value) = candidates[0].get(field) {
                        receipt.insert(field.into(), value.clone());
                    }
                }
                receipt
            } else {
                json!({"protocol":evidence::CONNECTOR_PROTOCOL,"browser_type":"chrome","status":"failed","connector_name":evidence::CONNECTOR_NAME,"connector_path":evidence::CONNECTOR_PATH,"chat_mode":"unverified","pro_mode":false,"action":"none","failed_stage":"evidence_invalid"}).as_object().expect("object literal").clone()
            }
        }
    };
    receipt.insert("session_id".into(), json!(session));
    receipt.insert("turn_id".into(), json!(turn));
    receipt.insert("probe_sha256".into(), json!(digest));
    write_receipt(kind, &receipt, data, session, turn)
        .map_err(|e| format!("evidence_hook_failed stage=write_failed: {e}"))?;
    Ok(None)
}

fn identity<'a>(payload: &'a Value, key: &str) -> Result<&'a str, String> {
    payload[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("evidence_hook_failed stage={key}_missing"))
}

fn trusted(kind: HookKind, payload: &Value, root: &Path) -> Result<bool, String> {
    let raw = &payload["tool_input"];
    let Some(code) = raw
        .as_str()
        .or_else(|| raw["code"].as_str())
        .or_else(|| raw["input"].as_str())
    else {
        return Ok(false);
    };
    let expected = match (kind, payload["tool_name"].as_str()) {
        (HookKind::Browser, Some("exec" | "functions.exec")) => {
            vec![evidence::canonical_browser_probe_code(root)]
        }
        (HookKind::Browser, Some("js" | "mcp__node_repl__js" | "node_repl.js")) => {
            vec![evidence::canonical_browser_inner_probe_code(root)]
        }
        (HookKind::Connector, Some("exec" | "functions.exec")) => vec![
            evidence::canonical_connector_probe_code(root),
            evidence::canonical_connector_retry_probe_code(root),
        ],
        (HookKind::Connector, Some("js" | "mcp__node_repl__js" | "node_repl.js")) => {
            vec![evidence::canonical_connector_inner_probe_code(root)]
        }
        _ => return Ok(false),
    };
    for expected in expected {
        let expected = expected.map_err(|error| error.to_string())?;
        if code == expected || code == format!("{expected}\n") || code == format!("{expected}\r\n")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn valid_browser(value: &Map<String, Value>) -> bool {
    if value.get("protocol").and_then(Value::as_str) != Some(evidence::BROWSER_EVIDENCE_PROTOCOL)
        || value.get("browser_type").and_then(Value::as_str) != Some("chrome")
    {
        return false;
    }
    match value.get("status").and_then(Value::as_str) {
        Some("available" | "unverified") => {
            value.get("can_report_unavailable").and_then(Value::as_bool) == Some(false)
        }
        Some("unavailable") => {
            value.get("can_report_unavailable").and_then(Value::as_bool) == Some(true)
                && value.get("failed_stage").and_then(Value::as_str) == Some("select_chrome_retry")
                && value
                    .get("public_error")
                    .and_then(Value::as_str)
                    .is_some_and(|error| !error.is_empty())
        }
        _ => false,
    }
}

fn write_receipt(
    kind: HookKind,
    receipt: &Map<String, Value>,
    data: &Path,
    session: &str,
    turn: &str,
) -> Result<(), String> {
    let directory = data.join(kind.directory());
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!("{}.json", common::receipt_key(session, turn)));
    let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, receipt).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}
