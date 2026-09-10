use std::fs;
use std::path::{Path, PathBuf};

use cdr_pro::evidence::{
    CONNECTOR_NAME, CONNECTOR_PATH, CONNECTOR_PROBE_SHA256, CONNECTOR_PROTOCOL, EvidenceError,
    canonical_connector_inner_probe_code, canonical_connector_probe_code,
    canonical_connector_retry_probe_code, require_connector_verified,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn plugin_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("plugins/codex-discord-remote")
}

fn receipt_key(session_id: &str, turn_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(session_id.as_bytes());
    digest.update([0]);
    digest.update(turn_id.as_bytes());
    hex::encode(digest.finalize())
}

fn connector_evidence(status: &str) -> Value {
    if status == "verified" {
        json!({
            "protocol": CONNECTOR_PROTOCOL,
            "browser_type": "chrome",
            "status": "verified",
            "connector_name": CONNECTOR_NAME,
            "connector_path": CONNECTOR_PATH,
            "chat_mode": "chat",
            "pro_mode": true,
            "action": "attached"
        })
    } else {
        json!({
            "protocol": CONNECTOR_PROTOCOL,
            "browser_type": "chrome",
            "status": "failed",
            "connector_name": CONNECTOR_NAME,
            "connector_path": CONNECTOR_PATH,
            "chat_mode": "unverified",
            "pro_mode": false,
            "action": "none",
            "failed_stage": "connector_search"
        })
    }
}

fn write_transcript(codex_home: &Path, attempts: &[(&str, String, Option<Value>)]) {
    let path = codex_home
        .join("sessions/2026/08/31")
        .join("rollout-test-session-a.jsonl");
    fs::create_dir_all(path.parent().expect("transcript parent"))
        .expect("create transcript directory");
    let mut records = Vec::new();
    for (call_id, code, evidence) in attempts {
        records.push(json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "name": "functions.exec",
                "input": code,
                "call_id": call_id,
                "internal_chat_message_metadata_passthrough": {"turn_id": "turn-a"}
            }
        }));
        if let Some(evidence) = evidence {
            records.push(json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call_output",
                    "call_id": call_id,
                    "output": evidence.to_string(),
                    "internal_chat_message_metadata_passthrough": {"turn_id": "turn-a"}
                }
            }));
        }
    }
    let body = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path, format!("{body}\n")).expect("write connector transcript");
}

#[cfg(windows)]
#[test]
fn canonical_connector_code_matches_frozen_python_output() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/parity/pro_canonical_probe_windows.json"
    ))
    .expect("parse frozen Python fixture");
    let root = Path::new(fixture["plugin_root"].as_str().expect("fixture root"));
    assert_eq!(
        canonical_connector_inner_probe_code(root).expect("inner connector code"),
        fixture["connector_inner"].as_str().expect("inner fixture")
    );
    assert_eq!(
        canonical_connector_probe_code(root).expect("outer connector code"),
        fixture["connector_outer"].as_str().expect("outer fixture")
    );
    assert_eq!(
        canonical_connector_retry_probe_code(root).expect("retry connector code"),
        fixture["connector_retry"].as_str().expect("retry fixture")
    );
}

#[test]
fn exact_verified_connector_receipt_passes() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let directory = temp.path().join("pro-connector-evidence");
    fs::create_dir(&directory).expect("create receipt directory");
    let mut receipt = connector_evidence("verified");
    let values = receipt.as_object_mut().expect("receipt object");
    values.insert("session_id".into(), json!("session-a"));
    values.insert("turn_id".into(), json!("turn-a"));
    values.insert("probe_sha256".into(), json!(CONNECTOR_PROBE_SHA256));
    fs::write(
        directory.join(format!("{}.json", receipt_key("session-a", "turn-a"))),
        serde_json::to_vec(&receipt).expect("serialize receipt"),
    )
    .expect("write receipt");

    require_connector_verified(
        "session-a",
        "turn-a",
        temp.path(),
        &temp.path().join("missing-home"),
        &plugin_root(),
    )
    .expect("exact verified connector receipt passes");
}

#[test]
fn last_canonical_attempt_wins_and_corrupt_receipt_cannot_be_bypassed() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let home = temp.path().join("home");
    let root = plugin_root();
    let primary = canonical_connector_probe_code(&root).expect("primary code");
    let retry = canonical_connector_retry_probe_code(&root).expect("retry code");
    write_transcript(
        &home,
        &[
            ("call-a", primary, Some(connector_evidence("verified"))),
            ("call-b", retry, Some(connector_evidence("failed"))),
        ],
    );
    assert!(matches!(
        require_connector_verified(
            "session-a",
            "turn-a",
            &temp.path().join("missing-data"),
            &home,
            &root,
        ),
        Err(EvidenceError::ConnectorUnavailable { .. })
    ));

    let receipt_dir = temp.path().join("data/pro-connector-evidence");
    fs::create_dir_all(&receipt_dir).expect("create corrupt receipt directory");
    fs::write(
        receipt_dir.join(format!("{}.json", receipt_key("session-a", "turn-a"))),
        "{",
    )
    .expect("write corrupt receipt");
    assert!(matches!(
        require_connector_verified(
            "session-a",
            "turn-a",
            &temp.path().join("data"),
            &home,
            &root,
        ),
        Err(EvidenceError::ConnectorUnavailable { .. })
    ));
}

#[test]
fn verified_retry_supersedes_failure_but_incomplete_retry_fails_closed() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let home = temp.path().join("home");
    let root = plugin_root();
    let primary = canonical_connector_probe_code(&root).expect("primary code");
    let retry = canonical_connector_retry_probe_code(&root).expect("retry code");
    write_transcript(
        &home,
        &[
            (
                "call-a",
                primary.clone(),
                Some(connector_evidence("failed")),
            ),
            (
                "call-b",
                retry.clone(),
                Some(connector_evidence("verified")),
            ),
        ],
    );
    require_connector_verified(
        "session-a",
        "turn-a",
        &temp.path().join("missing-data"),
        &home,
        &root,
    )
    .expect("verified retry is authoritative");

    write_transcript(
        &home,
        &[
            ("call-a", primary, Some(connector_evidence("verified"))),
            ("call-b", retry, None),
        ],
    );
    assert!(matches!(
        require_connector_verified(
            "session-a",
            "turn-a",
            &temp.path().join("missing-data"),
            &home,
            &root,
        ),
        Err(EvidenceError::ConnectorUnavailable { .. })
    ));
}
