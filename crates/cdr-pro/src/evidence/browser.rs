use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use url::Url;

use super::common::{
    embedded_objects, payload_turn_id, probe_integrity_ok, read_json_object, receipt_key,
    response_payload, transcript_paths,
};
use super::{
    BROWSER_EVIDENCE_PROTOCOL, BROWSER_PROBE_SHA256, EvidenceError, Result, chrome_unavailable,
};

const PROBE_PATH: &str = "skills/ask-chatgpt-pro/scripts/browser_evidence_probe.mjs";

pub fn canonical_browser_inner_probe_code(plugin_root: &Path) -> Result<String> {
    let path = plugin_root.join(PROBE_PATH);
    let uri = Url::from_file_path(&path)
        .map_err(|()| EvidenceError::InvalidFilePath(path.display().to_string()))?;
    let encoded = serde_json::to_string(uri.as_str())
        .map_err(|error| EvidenceError::InvalidFilePath(format!("{}: {error}", path.display())))?;
    Ok(format!(
        "nodeRepl.write(JSON.stringify(await (await import({encoded})).probeChrome({{ agent, chrome }})));"
    ))
}

pub fn canonical_browser_probe_code(plugin_root: &Path) -> Result<String> {
    let input = serde_json::to_string(&json!({
        "code": canonical_browser_inner_probe_code(plugin_root)?,
        "title": "Verify Chrome",
    }))
    .map_err(|error| EvidenceError::InvalidFilePath(error.to_string()))?;
    Ok(format!(
        "const browserEvidenceResult = await tools.mcp__node_repl__js({input});\nfor (const item of browserEvidenceResult.content ?? []) {{\n  if (item.type === \"text\") text(item.text);\n}}"
    ))
}

pub fn require_browser_available(
    session_id: &str,
    turn_id: &str,
    plugin_data: &Path,
) -> Result<()> {
    require_evidence(read_receipt(session_id, turn_id, plugin_data))
}

pub fn require_browser_available_with_transcript(
    session_id: &str,
    turn_id: &str,
    plugin_data: &Path,
    codex_home: &Path,
    plugin_roots: &[PathBuf],
) -> Result<()> {
    let evidence = read_receipt(session_id, turn_id, plugin_data)
        .or_else(|| read_transcript(session_id, turn_id, codex_home, plugin_roots));
    require_evidence(evidence)
}

fn require_evidence(evidence: Option<Map<String, Value>>) -> Result<()> {
    let evidence = evidence.ok_or_else(|| {
        chrome_unavailable(
            "Pro turn completed without verified Chrome evidence for the exact Codex session and turn.",
        )
    })?;
    if evidence.get("status").and_then(Value::as_str) != Some("available") {
        return Err(chrome_unavailable(
            "Pro turn did not acquire Chrome for the exact turn.",
        ));
    }
    if evidence
        .get("can_report_unavailable")
        .and_then(Value::as_bool)
        != Some(false)
    {
        return Err(chrome_unavailable(
            "Verified Chrome evidence had an invalid availability flag.",
        ));
    }
    Ok(())
}

fn read_receipt(session_id: &str, turn_id: &str, plugin_data: &Path) -> Option<Map<String, Value>> {
    if session_id.is_empty() || turn_id.is_empty() {
        return None;
    }
    let path = plugin_data
        .join("browser-evidence")
        .join(format!("{}.json", receipt_key(session_id, turn_id)));
    let receipt = read_json_object(&path)?;
    valid_receipt(&receipt, session_id, turn_id).then_some(receipt)
}

fn valid_receipt(values: &Map<String, Value>, session_id: &str, turn_id: &str) -> bool {
    values.get("protocol").and_then(Value::as_str) == Some(BROWSER_EVIDENCE_PROTOCOL)
        && values.get("session_id").and_then(Value::as_str) == Some(session_id)
        && values.get("turn_id").and_then(Value::as_str) == Some(turn_id)
        && values.get("browser_type").and_then(Value::as_str) == Some("chrome")
        && values.get("probe_sha256").and_then(Value::as_str) == Some(BROWSER_PROBE_SHA256)
        && values
            .get("can_report_unavailable")
            .is_some_and(Value::is_boolean)
}

fn read_transcript(
    session_id: &str,
    turn_id: &str,
    codex_home: &Path,
    roots: &[PathBuf],
) -> Option<Map<String, Value>> {
    let trusted = roots
        .iter()
        .filter(|root| probe_integrity_ok(root, Path::new(PROBE_PATH), BROWSER_PROBE_SHA256))
        .filter_map(|root| canonical_browser_probe_code(root).ok())
        .flat_map(|code| [code.clone(), format!("{code}\n"), format!("{code}\r\n")])
        .collect::<HashSet<_>>();
    let mut calls = HashSet::new();
    let mut latest = None;
    for path in transcript_paths(codex_home, session_id) {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in content.lines() {
            let Some(payload) = response_payload(line) else {
                continue;
            };
            if payload_turn_id(&payload) != Some(turn_id) {
                continue;
            }
            let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
                continue;
            };
            match payload.get("type").and_then(Value::as_str) {
                Some("custom_tool_call")
                    if matches!(
                        payload.get("name").and_then(Value::as_str),
                        Some("exec" | "functions.exec")
                    ) && payload
                        .get("input")
                        .and_then(Value::as_str)
                        .is_some_and(|input| trusted.contains(input)) =>
                {
                    calls.insert(call_id.to_owned());
                }
                Some("custom_tool_call_output") if calls.contains(call_id) => {
                    latest = payload
                        .get("output")
                        .and_then(|raw| embedded_objects(raw).into_iter().find(valid_browser));
                }
                _ => {}
            }
        }
    }
    latest
}

fn valid_browser(values: &Map<String, Value>) -> bool {
    let status = values.get("status").and_then(Value::as_str);
    values.get("protocol").and_then(Value::as_str) == Some(BROWSER_EVIDENCE_PROTOCOL)
        && values.get("browser_type").and_then(Value::as_str) == Some("chrome")
        && matches!(status, Some("available" | "unavailable" | "unverified"))
        && values
            .get("can_report_unavailable")
            .and_then(Value::as_bool)
            == Some(status == Some("unavailable"))
}
