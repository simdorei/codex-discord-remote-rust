#![cfg(windows)]
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

struct RecoveryFixture {
    root: tempfile::TempDir,
    db: PathBuf,
    backup: PathBuf,
    before: Vec<u8>,
    backup_before: Vec<u8>,
    state: PathBuf,
}

impl RecoveryFixture {
    fn new(handoff_exit: i32) -> Self {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("current.sqlite");
        drop(cdr_store::schema::open_initialized(&db).unwrap());
        let backup = root.path().join("before-execution.sqlite");
        // A closed, private fixture snapshot, not an operating backup.
        std::fs::copy(&db, &backup).unwrap();
        let backup_before = std::fs::read(&backup).unwrap();
        fixture::dispatching(&db, "no-rewind-resident");
        fixture::pending(&db, "later-pending", "thread-b", 1);
        let before = std::fs::read(&db).unwrap();
        assert_ne!(
            before, backup_before,
            "fixture must contain newer execution evidence"
        );
        assert!(cdr_store::async_resolution::admission_held(&db, "thread-b").unwrap());
        let watchdog = root.path().join("fixture-watchdog.ps1");
        std::fs::write(&watchdog,format!(
            "param([string]$RepoRoot,[string]$BinaryPath,[string]$RecoverDeploymentStatePath)\n$ErrorActionPreference='Stop'\n[IO.File]::AppendAllText((Join-Path $RepoRoot 'handoffs.log'),'handoff')\nexit {handoff_exit}\n"
        )).unwrap();
        let state = root.path().join("operation.json");
        std::fs::write(
            &state,
            serde_json::to_vec(&serde_json::json!({
                "Version":1,"LockPath":root.path().join("operation.lock"),
                "Watchdog":watchdog,"RepoRoot":root.path(),
                "BinaryPath":root.path().join("not-launched.exe")
            }))
            .unwrap(),
        )
        .unwrap();
        Self {
            root,
            db,
            backup,
            before,
            backup_before,
            state,
        }
    }

    fn invoke(&self, rewind: bool) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/candidates/async-recovery-v1/Recover-CdrDeployment.ps1");
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(script)
            .arg("-StatePath")
            .arg(&self.state);
        if rewind {
            command.arg("-RestoreDatabasePath").arg(&self.backup);
        }
        command.current_dir(self.root.path()).output().unwrap()
    }

    fn handoffs(&self) -> usize {
        std::fs::read_to_string(self.root.path().join("handoffs.log"))
            .unwrap_or_default()
            .matches("handoff")
            .count()
    }

    fn version(&self, version: Option<i64>) {
        let mut state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&self.state).unwrap()).unwrap();
        if let Some(version) = version {
            state["Version"] = version.into();
        } else {
            state.as_object_mut().unwrap().remove("Version");
        }
        std::fs::write(&self.state, serde_json::to_vec(&state).unwrap()).unwrap();
    }

    fn preserved(&self) {
        assert_eq!(std::fs::read(&self.db).unwrap(), self.before);
        assert_eq!(std::fs::read(&self.backup).unwrap(), self.backup_before);
        assert!(cdr_store::async_resolution::admission_held(&self.db, "thread-b").unwrap());
    }
}

#[test]
fn public_recovery_refuses_database_rewind_option_before_handoff() {
    let fixture = RecoveryFixture::new(0);
    let output = fixture.invoke(true);
    fixture.preserved();
    assert_eq!(
        fixture.handoffs(),
        0,
        "unsupported rewind argument must not start recovery: {output:?}"
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("RestoreDatabasePath"),
        "{output:?}"
    );
}

#[test]
fn supported_recovery_handoff_preserves_newer_execution_evidence_and_snapshot() {
    let fixture = RecoveryFixture::new(0);
    let output = fixture.invoke(false);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(fixture.handoffs(), 1);
    fixture.preserved();
}

#[test]
fn failed_recovery_handoff_does_not_rewind_or_erase_execution_evidence() {
    let fixture = RecoveryFixture::new(7);
    let output = fixture.invoke(false);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(fixture.handoffs(), 1);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Recovery watchdog failed"),
        "{output:?}"
    );
    fixture.preserved();
}

#[test]
fn public_recovery_still_accepts_legacy_state_without_version_without_rewinding() {
    let fixture = RecoveryFixture::new(0);
    fixture.version(None);
    let output = fixture.invoke(false);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(fixture.handoffs(), 1);
    fixture.preserved();
}

#[test]
fn public_recovery_rejects_newer_state_before_handoff_without_rewinding() {
    let fixture = RecoveryFixture::new(0);
    fixture.version(Some(2));
    let output = fixture.invoke(false);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(fixture.handoffs(), 0);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("maintenance_v2_or_unknown"),
        "{output:?}"
    );
    fixture.preserved();
}

#[test]
fn native_public_entrypoints_reject_unsupported_restore_without_touching_current_store() {
    let fixture = RecoveryFixture::new(0);
    for prefix in [vec![], vec!["--admin", "backup-store"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
            .args(prefix)
            .arg("--restore-store")
            .arg(&fixture.backup)
            .current_dir(fixture.root.path())
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unknown"),
            "{output:?}"
        );
        fixture.preserved();
    }
    assert_eq!(fixture.handoffs(), 0);
}
