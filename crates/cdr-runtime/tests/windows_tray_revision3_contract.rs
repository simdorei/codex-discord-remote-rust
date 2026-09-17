#![cfg(windows)]
//! Source-connected, offline tray revision 3 regression tests. No live bot or UI.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for name in [
        "codex-discord-tray.ps1",
        "codex-discord-tray-runtime.ps1",
        "codex-discord-tray-restart-runtime.ps1",
    ] {
        fs::copy(repo().join(name), root.path().join(name)).unwrap();
    }
    root
}

fn command(root: &Path, file: &str, case: &str) -> Command {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-STA", "-File"])
        .arg(
            repo()
                .join("crates/cdr-runtime/tests/fixtures/tray")
                .join(file),
        )
        .args(["-Case", case])
        .env("TRAY_CONTRACT_ROOT", root)
        .env("TRAY_CONTRACT_SOURCE", repo())
        .env("CODEX_DISCORD_RUNTIME", "rust")
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn output(mut cmd: Command) -> Output {
    let mut child = cmd.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("offline tray fixture timed out");
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

fn cases(file: &str, variants: &[&str]) {
    for case in variants {
        let root = fixture();
        let result = output(command(root.path(), file, case));
        assert!(
            result.status.success(),
            "{file}/{case}\n{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn absence_and_definite_other_process_remain_stopped() {
    cases(
        "revision3-observation.ps1",
        &["missing_lock", "wrong_path", "pid_absent", "valid"],
    );
}

#[test]
fn malformed_and_duplicate_lock_fields_are_unknown() {
    cases(
        "revision3-observation.ps1",
        &[
            "empty",
            "incomplete",
            "zero",
            "overflow",
            "duplicate",
            "mixed_duplicate",
            "directory_lock",
        ],
    );
}

#[test]
fn low_level_observation_failures_never_become_stopped() {
    cases(
        "revision3-observation.ps1",
        &[
            "read_denied",
            "item_denied",
            "process_denied",
            "process_nonterminating",
            "process_empty",
            "path_empty",
            "time_empty",
            "wrong_id",
            "getter_error",
            "recovery",
        ],
    );
}

#[test]
fn actual_once_exit_preserves_unknown_code_two() {
    let root = fixture();
    let result = output(command(root.path(), "revision3-observation.ps1", "once"));
    assert_eq!(
        result.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("unknown"));
}

#[test]
fn explicit_launch_intent_is_consumed_at_most_once() {
    cases(
        "revision3-delivery.ps1",
        &[
            "ordinary",
            "completed",
            "maintenance_completed",
            "empty_claim",
            "unknown_spawn",
            "owner_closed",
            "noninteractive",
        ],
    );
}

#[test]
fn stale_legacy_foreign_or_new_control_observations_cannot_launch() {
    cases(
        "revision3-delivery.ps1",
        &[
            "no_intent",
            "wrong_root",
            "wrong_child",
            "markers",
            "marker_race",
            "identity_race",
        ],
    );
}

#[test]
fn two_concurrent_watchdogs_share_an_atomic_ui_attempt() {
    let root = fixture();
    let first = command(root.path(), "revision3-delivery.ps1", "worker");
    let second = command(root.path(), "revision3-delivery.ps1", "worker");
    let a = thread::spawn(move || output(first));
    let b = thread::spawn(move || output(second));
    for result in [a.join().unwrap(), b.join().unwrap()] {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let attempts = fs::read_to_string(root.path().join("fixture-spawns.log")).unwrap();
    assert_eq!(attempts.lines().count(), 1);
}

#[test]
fn real_maintenance_engine_launch_and_receipt_never_enter_ui_helper() {
    cases("revision3-maintenance.ps1", &["slow", "throw", "unknown"]);
}

#[test]
fn restart_receipt_needs_same_invocation_exact_launch_hint() {
    cases("revision3-maintenance.ps1", &["restart_intent"]);
}

#[test]
fn real_watchdog_control_modes_keep_ui_outside_control_and_maintenance() {
    cases(
        "revision3-entry.ps1",
        &[
            "maintenance",
            "recovery",
            "complete",
            "pending_restart",
            "prepare",
            "check",
            "dry_run",
            "disabled",
            "stop",
            "failure",
            "ordinary",
            "ordinary_start",
            "ordinary_throw",
        ],
    );
}
