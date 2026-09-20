#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

const MARKER: &[u8] = b"operator_disabled\n";

fn prepare(root: &Path) -> (PathBuf, PathBuf) {
    let fixture = wrapper::prepare(root);
    (fixture.target, fixture.debug_harness)
}

fn sha256(path: &Path) -> String {
    hex::encode_upper(Sha256::digest(fs::read(path).unwrap()))
}

fn command(root: &Path, target: &Path, harness: &Path, expected: Option<&str>) -> Command {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(root.join("codex-discord-rust-soak.ps1"))
        .arg("-RepoRoot")
        .arg(root)
        .arg("-OutputDirectory")
        .arg(root.join("evidence"))
        .arg("-HarnessPath")
        .arg(harness)
        .args(["-SkipBuild", "-DurationSeconds", "1"])
        .env("CARGO_TARGET_DIR", target)
        .env_remove("DISCORD_BOT_TOKEN")
        .env_remove("DISCORD_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(value) = expected {
        command.arg("-ExpectedHarnessSha256").arg(value);
    }
    command
}

#[rustfmt::skip]
fn wait_for_output(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let pid = child.id().to_string();
            let _ = Command::new("taskkill").args(["/PID", &pid, "/T", "/F"]).output();
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("wrapper did not fail closed within deadline\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        }
        thread::sleep(Duration::from_millis(25));
    }
    child.wait_with_output().unwrap()
}

fn summary(root: &Path) -> Value {
    let path = fs::read_dir(root.join("evidence"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.ends_with(".summary.json") && !name.ends_with(".harness.summary.json")
        })
        .expect("fixed-shape failure summary");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[rustfmt::skip]
fn assert_prelaunch_failure(root: &Path, evidence: &Value, code: &str, message: &str) {
    assert_eq!(evidence["schema"], "cdr.windows-soak.summary.v3");
    assert!(evidence.get("status").is_none());
    assert_eq!(evidence["operational_status"], "integrity_failed");
    assert_eq!(evidence["final_eligibility"]["status"], "failed");
    assert_eq!(evidence["final_eligibility"]["eligible"], false);
    assert_eq!(evidence["source_provenance"]["primary_failure"], json!({
        "stage": "harness_hash_verification", "code": code, "message": message
    }));
    for field in ["harness_pid", "harness_started_at_utc", "child_exit_code"] {
        assert!(evidence.get(field).is_some_and(Value::is_null), "{field}");
    }
    let provenance = evidence["harness_provenance"].as_object().unwrap();
    for field in [
        "canonical_path",
        "expected_sha256",
        "sha256_before",
        "sha256_after",
        "length_before_bytes",
        "length_after_bytes",
        "unchanged",
        "pid",
        "process_started_at_utc",
        "process_start_ticks",
        "actual_process_image_path",
        "verified",
    ] {
        assert!(
            provenance.contains_key(field),
            "missing provenance field {field}"
        );
    }
    for field in [
        "sha256_after",
        "length_after_bytes",
        "unchanged",
        "pid",
        "process_started_at_utc",
        "process_start_ticks",
        "actual_process_image_path",
    ] {
        assert!(
            provenance[field].is_null(),
            "{field} must be null before launch"
        );
    }
    assert_eq!(provenance["verified"], false);
    assert!(!fs::read_dir(root.join("evidence")).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .to_string_lossy()
            .ends_with(".memory.jsonl")
    }));
    assert_eq!(
        fs::read(root.join(".codex_discord_bot.disabled")).unwrap(),
        MARKER
    );
}

#[test]
#[rustfmt::skip]
fn missing_expected_hash_has_no_default_and_fails_before_launch() {
    let temp = tempfile::tempdir().unwrap();
    let (target, harness) = prepare(temp.path());
    let actual_hash = sha256(&harness);
    let output = wait_for_output(
        command(temp.path(), &target, &harness, None)
            .spawn()
            .unwrap(),
    );
    assert!(!output.status.success());
    let evidence = summary(temp.path());
    assert_prelaunch_failure(temp.path(), &evidence, "expected_hash_missing",
        "ExpectedHarnessSha256 is required for offline soak evidence");
    let provenance = &evidence["harness_provenance"];
    assert!(provenance["expected_sha256"].is_null());
    assert_eq!(provenance["sha256_before"], actual_hash);
    assert_eq!(
        provenance["length_before_bytes"],
        fs::metadata(&harness).unwrap().len()
    );
}

#[test]
#[rustfmt::skip]
fn malformed_expected_hash_fails_before_launch_with_partial_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let (target, harness) = prepare(temp.path());
    let malformed = "not-a-sha256";
    let output = wait_for_output(
        command(temp.path(), &target, &harness, Some(malformed))
            .spawn()
            .unwrap(),
    );
    assert!(!output.status.success());
    let evidence = summary(temp.path());
    assert_prelaunch_failure(temp.path(), &evidence, "expected_hash_malformed",
        "ExpectedHarnessSha256 must contain exactly 64 hexadecimal characters");
    let provenance = &evidence["harness_provenance"];
    assert_eq!(
        provenance["expected_sha256"],
        malformed.to_ascii_uppercase()
    );
    assert_eq!(provenance["sha256_before"], sha256(&harness));
    assert_eq!(
        provenance["length_before_bytes"],
        fs::metadata(&harness).unwrap().len()
    );
}

#[test]
#[rustfmt::skip]
fn missing_disabled_marker_still_writes_fixed_shape_v3_failure() {
    let temp = tempfile::tempdir().unwrap();
    let (target, harness) = prepare(temp.path());
    fs::remove_file(temp.path().join(".codex_discord_bot.disabled")).unwrap();
    let expected = sha256(&harness);
    let output = wait_for_output(
        command(temp.path(), &target, &harness, Some(&expected))
            .spawn()
            .unwrap(),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("operator disabled marker"));
    let evidence = summary(temp.path());
    assert_eq!(evidence["schema"], "cdr.windows-soak.summary.v3");
    assert!(evidence.get("status").is_none());
    assert_eq!(evidence["operational_status"], "runtime_failed");
    assert_eq!(evidence["final_eligibility"]["status"], "failed");
    assert_eq!(evidence["final_eligibility"]["eligible"], false);
    assert_eq!(evidence["source_provenance"]["primary_failure"], json!({
        "stage": "prepare", "code": "prepare_failed", "message": format!(
            "Offline soak requires the existing operator disabled marker: {}",
            temp.path().join(".codex_discord_bot.disabled").display())
    }));
    for field in ["harness_pid", "harness_started_at_utc", "child_exit_code"] {
        assert!(evidence.get(field).is_some_and(Value::is_null), "{field}");
    }
    let provenance = &evidence["harness_provenance"];
    assert!(provenance.is_object());
    assert_eq!(provenance["expected_sha256"], expected);
    assert!(provenance["canonical_path"].is_null());
    assert!(provenance["sha256_before"].is_null());
    assert_eq!(provenance["verified"], false);
    assert!(
        !fs::read_dir(temp.path().join("evidence"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .path()
                .to_string_lossy()
                .ends_with(".memory.jsonl"))
    );
}
