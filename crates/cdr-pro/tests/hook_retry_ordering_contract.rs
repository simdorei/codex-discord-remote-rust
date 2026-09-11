use cdr_pro::{
    evidence::{self, CONNECTOR_NAME, CONNECTOR_PATH, CONNECTOR_PROBE_SHA256, CONNECTOR_PROTOCOL},
    hooks::{self, HookKind},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn plugin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/codex-discord-remote")
        .canonicalize()
        .unwrap()
}
fn receipt_path(data: &Path) -> PathBuf {
    data.join("pro-connector-evidence").join(format!(
        "{}.json",
        hex::encode(Sha256::digest(b"session-a\0turn-a"))
    ))
}
fn evidence(verified: bool) -> Value {
    if verified {
        json!({"protocol":CONNECTOR_PROTOCOL,"browser_type":"chrome","status":"verified","connector_name":CONNECTOR_NAME,"connector_path":CONNECTOR_PATH,"chat_mode":"chat","pro_mode":true,"action":"attached"})
    } else {
        json!({"protocol":CONNECTOR_PROTOCOL,"browser_type":"chrome","status":"failed","connector_name":CONNECTOR_NAME,"connector_path":CONNECTOR_PATH,"chat_mode":"unverified","pro_mode":false,"action":"none","failed_stage":"connector_search"})
    }
}
fn payload(root: &Path, retry: bool, body: &str) -> Value {
    json!({"hook_event_name":"PostToolUse","session_id":"session-a","turn_id":"turn-a","tool_name":"functions.exec","tool_input":if retry{evidence::canonical_connector_retry_probe_code(root).unwrap()}else{evidence::canonical_connector_probe_code(root).unwrap()},"tool_response":body})
}
fn verify(data: &Path, home: &Path) -> bool {
    evidence::require_connector_verified("session-a", "turn-a", data, home, &plugin()).is_ok()
}
fn transcript(home: &Path, attempts: &[Value]) {
    let path = home.join("sessions/2026/08/25/rollout-test-session-a.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut records = Vec::new();
    for (index, attempt) in attempts.iter().enumerate() {
        let id = format!("call-{index}");
        records.push(json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"functions.exec","input":attempt["tool_input"],"call_id":id,"internal_chat_message_metadata_passthrough":{"turn_id":"turn-a"}}}));
        records.push(json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":id,"output":attempt["tool_response"],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-a"}}}));
    }
    fs::write(
        path,
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
}
#[test]
fn failed_primary_then_verified_retry_replaces_the_receipt() {
    let data = tempfile::tempdir().unwrap();
    let root = plugin();
    for success in [false, true] {
        hooks::handle(
            HookKind::Connector,
            &payload(&root, success, &evidence(success).to_string()),
            &root,
            data.path(),
        )
        .unwrap();
        assert_eq!(verify(data.path(), &data.path().join("absent")), success);
    }
}
#[test]
fn invalid_duplicate_or_failed_retry_overwrites_prior_success() {
    let root = plugin();
    let success = evidence(true).to_string();
    for body in [
        "not evidence".into(),
        format!("{success}\n{success}"),
        evidence(false).to_string(),
    ] {
        let data = tempfile::tempdir().unwrap();
        hooks::handle(
            HookKind::Connector,
            &payload(&root, false, &success),
            &root,
            data.path(),
        )
        .unwrap();
        hooks::handle(
            HookKind::Connector,
            &payload(&root, true, &body),
            &root,
            data.path(),
        )
        .unwrap();
        assert!(!verify(data.path(), &data.path().join("absent")));
    }
}
#[test]
fn modified_retry_or_inner_source_cannot_write_a_receipt() {
    let root = plugin();
    for inner in [false, true] {
        for modified in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let mut value = payload(&root, true, &evidence(true).to_string());
            if inner {
                value["tool_name"] = json!("mcp__node_repl__js");
                value["tool_input"] =
                    json!(evidence::canonical_connector_inner_probe_code(&root).unwrap());
            }
            if modified {
                value["tool_input"] = json!(format!(
                    "{}\ntext('forged');",
                    value["tool_input"].as_str().unwrap()
                ));
            }
            hooks::handle(HookKind::Connector, &value, &root, data.path()).unwrap();
            assert_eq!(receipt_path(data.path()).is_file(), !modified);
        }
    }
}
#[test]
fn stale_success_without_a_committed_retry_receipt_cannot_override_the_transcript() {
    let data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let root = plugin();
    let primary = payload(&root, false, &evidence(true).to_string());
    let retry = payload(&root, true, "not evidence");
    hooks::handle(HookKind::Connector, &primary, &root, data.path()).unwrap();
    let before = fs::read(receipt_path(data.path())).unwrap();
    // Same boundary as the legacy mocked write failure: the old receipt remains.
    transcript(home.path(), &[primary, retry]);
    assert!(!verify(data.path(), home.path()));
    assert_eq!(fs::read(receipt_path(data.path())).unwrap(), before);
}
#[test]
fn delayed_primary_callback_cannot_override_a_failed_retry_in_transcript() {
    let data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let root = plugin();
    let primary = payload(&root, false, &evidence(true).to_string());
    let retry = payload(&root, true, &evidence(false).to_string());
    transcript(home.path(), &[primary.clone(), retry.clone()]);
    for value in [&primary, &retry, &primary] {
        hooks::handle(HookKind::Connector, value, &root, data.path()).unwrap();
    }
    assert!(!verify(data.path(), home.path()));
}
#[test]
fn invalid_retry_has_complete_failed_receipt_shape() {
    let data = tempfile::tempdir().unwrap();
    let root = plugin();
    for (retry, body) in [
        (false, evidence(true).to_string()),
        (true, "not evidence".into()),
    ] {
        hooks::handle(
            HookKind::Connector,
            &payload(&root, retry, &body),
            &root,
            data.path(),
        )
        .unwrap();
    }
    let actual: Value =
        serde_json::from_slice(&fs::read(receipt_path(data.path())).unwrap()).unwrap();
    let mut expected = evidence(false);
    expected["failed_stage"] = json!("evidence_invalid");
    expected["session_id"] = json!("session-a");
    expected["turn_id"] = json!("turn-a");
    expected["probe_sha256"] = json!(CONNECTOR_PROBE_SHA256);
    assert_eq!(actual, expected);
}
