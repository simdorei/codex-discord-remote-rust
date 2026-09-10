use std::cell::RefCell;
use std::fs;

use cdr_pro::diagnostics::{DiagnosticCode, DiagnosticStage, ProError, RuntimeDiagnostic};
use cdr_pro::preflight::{
    ResidentSnapshot, RuntimeStatus, expected_remote_plugin_version, recover_stale, verify_runtime,
};
use serde_json::json;

fn inventory(remote_version: &serde_json::Value) -> String {
    json!({
        "installed": [
            {
                "pluginId": "codex-discord-remote@codex-discord-remote",
                "installed": true,
                "enabled": true,
                "version": remote_version
            },
            {
                "pluginId": "chrome@openai-bundled",
                "installed": true,
                "enabled": true,
                "version": "9.8.7"
            }
        ]
    })
    .to_string()
}

fn healthy_resident() -> ResidentSnapshot {
    ResidentSnapshot {
        generation: 8,
        healthy: true,
        accepting: true,
        plugin_runtime_fingerprint: Some("fingerprint".into()),
        plugin_runtime_error: None,
    }
}

#[test]
fn valid_inventory_and_fresh_resident_pass_preflight() {
    let status = verify_runtime(
        &inventory(&json!("1.2.3")),
        "1.2.3",
        &healthy_resident(),
        "fingerprint",
    )
    .expect("valid Pro preflight");
    assert_eq!(
        status,
        RuntimeStatus {
            remote_plugin_version: "1.2.3".into(),
            browser_plugin_version: "9.8.7".into(),
            resident_generation: 8,
        }
    );
}

#[test]
fn invalid_or_stale_runtime_fails_closed_with_specific_code() {
    let cases = [
        (
            "not-json".to_owned(),
            DiagnosticCode::PluginInventoryInvalid,
        ),
        (
            inventory(&json!("wrong")),
            DiagnosticCode::RemotePluginVersionMismatch,
        ),
    ];
    for (raw, expected) in cases {
        let error = verify_runtime(&raw, "1.2.3", &healthy_resident(), "fingerprint")
            .expect_err("invalid preflight must fail");
        assert_eq!(error.diagnostic().expect("typed diagnostic").code, expected);
    }

    let error = verify_runtime(
        &inventory(&json!("1.2.3")),
        "1.2.3",
        &healthy_resident(),
        "changed",
    )
    .expect_err("stale resident must fail");
    assert_eq!(
        error.diagnostic().expect("typed diagnostic").code,
        DiagnosticCode::ResidentStale
    );
}

fn status() -> RuntimeStatus {
    RuntimeStatus {
        remote_plugin_version: "1.2.3".into(),
        browser_plugin_version: "9.8.7".into(),
        resident_generation: 8,
    }
}

fn failure(code: DiagnosticCode) -> ProError {
    ProError::Preflight(RuntimeDiagnostic {
        stage: DiagnosticStage::ResidentAppServer,
        code,
        public_message: "test failure".into(),
        recovery_action: "test recovery".into(),
        internal_detail: "test detail".into(),
    })
}

#[test]
fn stale_recovery_refreshes_once_then_retries_once() {
    let calls = RefCell::new(Vec::new());
    let checks = RefCell::new(0_u8);
    let result = recover_stale(
        || {
            calls.borrow_mut().push("check");
            *checks.borrow_mut() += 1;
            if *checks.borrow() == 1 {
                Err(failure(DiagnosticCode::ResidentStale))
            } else {
                Ok(status())
            }
        },
        || {
            calls.borrow_mut().push("refresh");
            Ok(true)
        },
    )
    .expect("stale runtime should recover");
    assert_eq!(result, status());
    assert_eq!(*calls.borrow(), ["check", "refresh", "check"]);
}

#[test]
fn recovery_fails_closed_without_loops_or_diagnostic_loss() {
    let refreshes = RefCell::new(0_u8);
    let error = recover_stale(
        || Err(failure(DiagnosticCode::PluginInventoryInvalid)),
        || {
            *refreshes.borrow_mut() += 1;
            Ok(true)
        },
    )
    .expect_err("non-stale failure");
    assert_eq!(*refreshes.borrow(), 0);
    assert_eq!(
        error.diagnostic().expect("diagnostic").code,
        DiagnosticCode::PluginInventoryInvalid
    );

    let error = recover_stale(
        || Err(failure(DiagnosticCode::ResidentStale)),
        || Err("resident refresh failed".into()),
    )
    .expect_err("refresh failure");
    let diagnostic = error.diagnostic().expect("diagnostic");
    assert_eq!(diagnostic.code, DiagnosticCode::ResidentStale);
    assert_eq!(diagnostic.public_message, "test failure");
    assert!(
        diagnostic
            .internal_detail
            .contains("resident refresh failed")
    );
}

#[test]
fn manifest_version_accepts_utf8_bom_and_rejects_invalid_input() {
    let directory = tempfile::tempdir().expect("tempdir");
    let manifest = directory.path().join("plugin.json");
    fs::write(&manifest, "\u{feff}{\"version\":\"1.2.3\"}").expect("write manifest");
    assert_eq!(
        expected_remote_plugin_version(&manifest).expect("BOM manifest"),
        "1.2.3"
    );

    fs::write(&manifest, "[]").expect("write invalid manifest");
    let invalid = expected_remote_plugin_version(&manifest).expect_err("object required");
    assert_eq!(
        invalid.diagnostic().expect("diagnostic").code,
        DiagnosticCode::RemoteManifestInvalid
    );

    let missing = expected_remote_plugin_version(&directory.path().join("missing.json"))
        .expect_err("missing manifest");
    assert_eq!(
        missing.diagnostic().expect("diagnostic").code,
        DiagnosticCode::RemoteManifestUnavailable
    );
}
