use std::fs;

use cdr_pro::evidence::{
    BROWSER_EVIDENCE_PROTOCOL, BROWSER_PROBE_SHA256, EvidenceError,
    canonical_browser_inner_probe_code, canonical_browser_probe_code, require_browser_available,
    require_browser_available_with_transcript,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn receipt_key(session_id: &str, turn_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(session_id.as_bytes());
    digest.update([0]);
    digest.update(turn_id.as_bytes());
    hex::encode(digest.finalize())
}

#[cfg(windows)]
#[test]
fn canonical_browser_code_matches_frozen_python_output() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/parity/pro_canonical_probe_windows.json"
    ))
    .expect("parse frozen Python fixture");
    let root = std::path::Path::new(
        fixture["plugin_root"]
            .as_str()
            .expect("fixture plugin root"),
    );
    assert_eq!(
        canonical_browser_inner_probe_code(root).expect("browser inner code"),
        fixture["browser_inner"]
            .as_str()
            .expect("browser inner fixture")
    );
    assert_eq!(
        canonical_browser_probe_code(root).expect("browser outer code"),
        fixture["browser_outer"]
            .as_str()
            .expect("browser outer fixture")
    );
}

#[test]
fn trusted_exact_turn_browser_transcript_is_accepted_without_receipt() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let codex_home = temp.path().join("codex-home");
    let transcript = codex_home
        .join("sessions/2026/08/31")
        .join("rollout-test-session-a.jsonl");
    fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("create transcript directory");
    let plugin_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("plugins/codex-discord-remote");
    let code = canonical_browser_probe_code(&plugin_root).expect("canonical browser code");
    let evidence = json!({
        "protocol": BROWSER_EVIDENCE_PROTOCOL,
        "browser_type": "chrome",
        "status": "available",
        "can_report_unavailable": false,
    });
    let records = [
        json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "call_id": "call-a",
                "name": "functions.exec",
                "input": code,
                "internal_chat_message_metadata_passthrough": {"turn_id": "turn-a"}
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call_output",
                "call_id": "call-a",
                "output": [{"type": "input_text", "text": evidence.to_string()}],
                "internal_chat_message_metadata_passthrough": {"turn_id": "turn-a"}
            }
        }),
    ];
    let body = records
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&transcript, format!("{body}\n")).expect("write trusted transcript");

    require_browser_available_with_transcript(
        "session-a",
        "turn-a",
        &temp.path().join("missing-data"),
        &codex_home,
        &[plugin_root],
    )
    .expect("trusted exact-turn transcript passes");
}

#[test]
fn exact_available_browser_receipt_passes_and_other_turn_fails_closed() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let directory = temp.path().join("browser-evidence");
    fs::create_dir(&directory).expect("create receipt directory");
    let receipt = json!({
        "protocol": BROWSER_EVIDENCE_PROTOCOL,
        "session_id": "session-a",
        "turn_id": "turn-a",
        "browser_type": "chrome",
        "status": "available",
        "can_report_unavailable": false,
        "probe_sha256": BROWSER_PROBE_SHA256,
    });
    fs::write(
        directory.join(format!("{}.json", receipt_key("session-a", "turn-a"))),
        serde_json::to_vec(&receipt).expect("serialize receipt"),
    )
    .expect("write receipt");

    require_browser_available("session-a", "turn-a", temp.path())
        .expect("exact available receipt passes");
    assert!(matches!(
        require_browser_available("session-a", "turn-b", temp.path()),
        Err(EvidenceError::ChromeUnavailable { .. })
    ));
}
