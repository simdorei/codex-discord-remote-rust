#![cfg(windows)]
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::create_dir(root.path().join("scripts")).unwrap();
    for file in [
        "scripts/Invoke-CdrMaintenance.ps1",
        "codex-discord-rust-control.ps1",
    ] {
        fs::copy(repo.join(file), root.path().join(file)).unwrap();
    }
    root
}
fn run(root: &Path, state: &Path) -> Output {
    Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts/Invoke-CdrMaintenance.ps1"))
        .arg("-StatePath")
        .arg(state)
        .args(["-ExpectedOperation", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"])
        .output()
        .unwrap()
}

#[test]
fn completed_receipt_requires_supported_shutdown_policy() {
    for policy in [None, Some("unknown"), Some("live-handshake-v1")] {
        let root = fixture();
        let state = root.path().join(".codex_discord_rust.maintenance.v2");
        let receipt = root
            .path()
            .join(".codex_discord_rust.maintenance.v2.completed");
        let mut record = json!({"Version":2,"Operation":"a".repeat(32),"Phase":"verified"});
        if let Some(policy) = policy {
            record["ShutdownPolicy"] = json!(policy);
        }
        let before = serde_json::to_vec(&record).unwrap();
        fs::write(&receipt, &before).unwrap();
        let output = run(root.path(), &state);
        if policy == Some("live-handshake-v1") {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("maintenance_previously_completed")
            );
        } else {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("shutdown_policy"));
        }
        assert_eq!(fs::read(receipt).unwrap(), before);
        assert!(!root.path().join(".codex_discord_rust.stop").exists());
    }
}

#[test]
fn old_operation_cannot_adopt_new_state_or_completed_receipt() {
    for completed in [false, true] {
        let root = fixture();
        let state = root.path().join(".codex_discord_rust.maintenance.v2");
        let record = if completed {
            root.path()
                .join(".codex_discord_rust.maintenance.v2.completed")
        } else {
            state.clone()
        };
        let before=serde_json::to_vec(&json!({"Version":2,"Operation":"b".repeat(32),"Phase":if completed{"verified"}else{"planned"},"Attempts":0})).unwrap();
        fs::write(&record, &before).unwrap();
        let output = run(root.path(), &state);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("expected_operation_mismatch"));
        assert_eq!(fs::read(record).unwrap(), before);
        assert!(!root.path().join(".codex_discord_rust.stop").exists());
    }
}

#[test]
fn obsolete_deployment_and_recovery_entries_refuse_v2_before_any_action() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for script in [
        "scripts/Recover-CdrDeployment.ps1",
        "scripts/Invoke-CdrDeployment.ps1",
    ] {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state.json");
        fs::write(
            &state,
            serde_json::to_vec(&json!({"Version":2,"RepoRoot":root.path()})).unwrap(),
        )
        .unwrap();
        let before = fs::read(&state).unwrap();
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(repo.join(script))
            .arg("-StatePath")
            .arg(&state)
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("maintenance_v2_or_unknown"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(state).unwrap(), before);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
