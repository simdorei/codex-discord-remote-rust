use std::fs;
use std::path::{Path, PathBuf};

use cdr_pro::evidence::{
    self, BROWSER_EVIDENCE_PROTOCOL, CONNECTOR_NAME, CONNECTOR_PATH, CONNECTOR_PROTOCOL,
};
use cdr_pro::hooks::{self, HookKind};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
        .join("plugins/codex-discord-remote")
}
fn key() -> String {
    hex::encode(Sha256::digest(b"session-a\0turn-a"))
}
fn browser(status: &str) -> Value {
    json!({"protocol":BROWSER_EVIDENCE_PROTOCOL,"browser_type":"chrome","status":status,"can_report_unavailable":status == "unavailable","failed_stage":"select_chrome_retry","public_error":"Selection failed after retry"})
}
fn connector() -> Value {
    json!({"protocol":CONNECTOR_PROTOCOL,"browser_type":"chrome","status":"verified","connector_name":CONNECTOR_NAME,"connector_path":CONNECTOR_PATH,"chat_mode":"chat","pro_mode":true,"action":"attached"})
}
fn payload(code: &str, response: &Value) -> Value {
    json!({"hook_event_name":"PostToolUse","tool_name":"functions.exec","tool_input":code,"tool_response":{"content":[{"type":"text","text":response.to_string()}]},"session_id":"session-a","turn_id":"turn-a","tool_use_id":"tool-a"})
}
fn stop(message: &str) -> Value {
    json!({"hook_event_name":"Stop","last_assistant_message":message,"session_id":"session-a","turn_id":"turn-a"})
}

#[test]
fn browser_hook_receipt_is_accepted_by_the_existing_rust_consumer() {
    let data = tempfile::tempdir().unwrap();
    let root = root();
    let code = evidence::canonical_browser_probe_code(&root).unwrap();
    assert_eq!(
        hooks::handle(
            HookKind::Browser,
            &payload(&code, &browser("available")),
            &root,
            data.path()
        )
        .unwrap(),
        None
    );
    evidence::require_browser_available("session-a", "turn-a", data.path()).unwrap();
    assert!(
        evidence::require_browser_available("session-a", "different-turn", data.path()).is_err()
    );
}

#[test]
fn unsupported_browser_claim_is_blocked_until_exact_turn_receipt_exists() {
    let data = tempfile::tempdir().unwrap();
    let root = root();
    for message in ["Chrome is unavailable.", "크롬 연결 불가입니다."] {
        assert_eq!(
            hooks::handle(HookKind::Browser, &stop(message), &root, data.path())
                .unwrap()
                .unwrap()["decision"],
            "block"
        );
    }
    assert!(
        hooks::handle(
            HookKind::Browser,
            &stop("Chrome bootstrap was not verified."),
            &root,
            data.path()
        )
        .unwrap()
        .is_none()
    );
    let code = evidence::canonical_browser_probe_code(&root).unwrap();
    hooks::handle(
        HookKind::Browser,
        &payload(&code, &browser("unavailable")),
        &root,
        data.path(),
    )
    .unwrap();
    assert!(
        hooks::handle(
            HookKind::Browser,
            &stop("Chrome is unavailable."),
            &root,
            data.path()
        )
        .unwrap()
        .is_none()
    );
    let mut wrong_turn = stop("Chrome is unavailable.");
    wrong_turn["turn_id"] = json!("different-turn");
    assert_eq!(
        hooks::handle(HookKind::Browser, &wrong_turn, &root, data.path())
            .unwrap()
            .unwrap()["decision"],
        "block"
    );
}

#[test]
fn connector_hook_handles_retry_and_rejects_multiple_evidence_objects() {
    let data = tempfile::tempdir().unwrap();
    let root = root();
    let code = evidence::canonical_connector_retry_probe_code(&root).unwrap();
    hooks::handle(
        HookKind::Connector,
        &payload(&code, &connector()),
        &root,
        data.path(),
    )
    .unwrap();
    evidence::require_connector_verified(
        "session-a",
        "turn-a",
        data.path(),
        &data.path().join("no-home"),
        &root,
    )
    .unwrap();
    let mut duplicate = payload(&code, &connector());
    duplicate["tool_response"] = json!([connector().to_string(), connector().to_string()]);
    hooks::handle(HookKind::Connector, &duplicate, &root, data.path()).unwrap();
    assert!(
        evidence::require_connector_verified(
            "session-a",
            "turn-a",
            data.path(),
            &data.path().join("no-home"),
            &root
        )
        .is_err()
    );
    let receipt: Value = serde_json::from_slice(
        &fs::read(
            data.path()
                .join("pro-connector-evidence")
                .join(format!("{}.json", key())),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["failed_stage"], "evidence_invalid");
}

#[test]
fn spoofed_or_modified_probe_cannot_create_a_success_receipt() {
    let data = tempfile::tempdir().unwrap();
    let root = root();
    hooks::handle(
        HookKind::Connector,
        &payload("console.log('trusted')", &connector()),
        &root,
        data.path(),
    )
    .unwrap();
    assert_eq!(fs::read_dir(data.path()).unwrap().count(), 0);
    let tampered = tempfile::tempdir().unwrap();
    let path = tampered
        .path()
        .join("skills/ask-chatgpt-pro/scripts/pro_connector_control.mjs");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "tampered").unwrap();
    let code = evidence::canonical_connector_probe_code(tampered.path()).unwrap();
    assert!(
        hooks::handle(
            HookKind::Connector,
            &payload(&code, &connector()),
            tampered.path(),
            data.path()
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(data.path()).unwrap().count(), 0);
}

#[test]
fn connector_write_error_and_missing_identity_are_explicit() {
    let data = tempfile::tempdir().unwrap();
    let root = root();
    let code = evidence::canonical_connector_probe_code(&root).unwrap();
    let mut missing = payload(&code, &connector());
    missing["turn_id"] = json!("");
    assert!(
        hooks::handle(HookKind::Connector, &missing, &root, data.path())
            .unwrap_err()
            .contains("turn_id_missing")
    );
    fs::write(data.path().join("pro-connector-evidence"), "obstruction").unwrap();
    assert!(
        hooks::handle(
            HookKind::Connector,
            &payload(&code, &connector()),
            &root,
            data.path()
        )
        .unwrap_err()
        .contains("write_failed")
    );
}
