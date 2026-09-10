#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

const MARKER: &[u8] = b"operator_disabled\n";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prepare(root: &Path) -> (PathBuf, PathBuf) {
    fs::write(root.join(".codex_discord_bot.disabled"), MARKER).unwrap();
    let target = root.join("fixture-target");
    let harness = target.join("debug/cdr-offline-soak.exe");
    fs::create_dir_all(harness.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), &harness).unwrap();
    (target, harness)
}

fn sha256(path: &Path) -> String {
    hex::encode_upper(Sha256::digest(fs::read(path).unwrap()))
}

#[rustfmt::skip]
fn soak_command(root: &Path, target: &Path, harness: &Path, hash: &str, duration: &str) -> Command {
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo_root().join("codex-discord-rust-soak.ps1"))
        .arg("-RepoRoot").arg(root)
        .arg("-OutputDirectory").arg(root.join("evidence"))
        .arg("-HarnessPath").arg(harness)
        .args(["-ExpectedHarnessSha256", hash, "-SkipBuild", "-DurationSeconds", duration,
            "-SampleIntervalSeconds", "0.1", "-WarmupSeconds", "0", "-HarnessExitGraceSeconds", "5",
            "-MaxSlopeBytesPerHour", "1000000000000000"])
        .env("CARGO_TARGET_DIR", target)
        .env_remove("DISCORD_BOT_TOKEN").env_remove("DISCORD_TOKEN");
    command
}

fn wrapper_summary(root: &Path) -> Value {
    let path = fs::read_dir(root.join("evidence"))
        .expect("wrapper summary evidence directory")
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.ends_with(".summary.json") && !name.ends_with(".harness.summary.json")
        })
        .expect("wrapper summary evidence");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[rustfmt::skip]
fn wait_for_memory(mut child: Child, root: &Path) -> (Child, Value) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let record = fs::read_dir(root.join("evidence")).ok().into_iter().flatten()
            .filter_map(Result::ok)
            .find(|entry| entry.path().to_string_lossy().ends_with(".memory.jsonl"))
            .and_then(|entry| fs::read_to_string(entry.path()).ok())
            .and_then(|text| text.lines().find(|line| !line.trim().is_empty()).map(str::to_owned))
            .and_then(|line| serde_json::from_str(&line).ok());
        if let Some(record) = record { return (child, record); }
        if child.try_wait().unwrap().is_some() {
            let output = child.wait_with_output().unwrap();
            panic!("wrapper exited before harness\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        }
        if Instant::now() >= deadline {
            let pid = child.id().to_string();
            let _ = Command::new("taskkill").args(["/PID", &pid, "/T", "/F"]).status();
            let _ = child.kill(); let _ = child.wait();
            panic!("timed out waiting for harness evidence");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[rustfmt::skip]
fn inspect_process(pid: u64) -> Value {
    let script = format!("$p=Get-Process -Id {pid} -ErrorAction Stop; [ordered]@{{pid=[long]$p.Id; \
        started_at_utc=$p.StartTime.ToUniversalTime().ToString('o'); \
        start_ticks=[long]$p.StartTime.ToUniversalTime().Ticks; \
        image_path=[IO.Path]::GetFullPath([string]$p.Path)}} | ConvertTo-Json -Compress");
    let output = Command::new("powershell.exe").args(["-NoProfile", "-Command", &script]).output().unwrap();
    assert!(output.status.success(), "process identity query failed");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "wrapper failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[rustfmt::skip]
fn wait_for_output(mut child: Child, timeout: Duration) -> Output {
    let deadline = Instant::now() + timeout;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let pid = child.id().to_string();
            let _ = Command::new("taskkill").args(["/PID", &pid, "/T", "/F"]).output();
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("wrapper timed out\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        }
        thread::sleep(Duration::from_millis(25));
    }
    child.wait_with_output().unwrap()
}

fn same_path(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        value
            .strip_prefix(r"\\?\")
            .unwrap_or(value)
            .replace('/', "\\")
            .to_ascii_lowercase()
    };
    normalize(left) == normalize(right)
}

#[test]
#[rustfmt::skip]
fn prov_01_short_run_records_independently_verified_harness_identity() {
    let temp = tempfile::tempdir().unwrap();
    let (target, harness) = prepare(temp.path());
    let expected_hash = sha256(&harness);
    let supplied_hash = expected_hash.to_ascii_lowercase();
    let expected_len = fs::metadata(&harness).unwrap().len();
    let canonical = fs::canonicalize(&harness).unwrap();
    let mut command = soak_command(temp.path(), &target, &harness, &supplied_hash, "2");
    let child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let (child, memory) = wait_for_memory(child, temp.path());
    let observed = inspect_process(memory["harness_pid"].as_u64().unwrap());
    let output = wait_for_output(child, Duration::from_secs(10));
    assert_success(&output);

    let summary = wrapper_summary(temp.path());
    let provenance = &summary["harness_provenance"]; assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3"); assert!(summary.get("status").is_none());
    assert!(provenance.is_object());
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3"); assert!(summary.get("status").is_none()); assert_eq!(summary["harness_summary"]["schema"], "cdr.offline-soak.summary.v1");
    assert_eq!(summary["harness_summary"]["assertions"]["offline_only"], true);
    assert_eq!(summary["mode"], "offline_fake_replay");
    assert_eq!(summary["operational_status"], "passed"); assert_eq!(summary["final_eligibility"]["status"], "ineligible"); assert_eq!(summary["final_eligibility"]["eligible"], false);
    assert!(same_path(provenance["canonical_path"].as_str().unwrap(), &canonical.to_string_lossy()));
    for field in ["expected_sha256", "sha256_before", "sha256_after"] {
        assert_eq!(provenance[field], expected_hash, "{field}");
    }
    assert_eq!(provenance["length_before_bytes"], expected_len);
    assert_eq!(provenance["length_after_bytes"], expected_len);
    assert_eq!(provenance["unchanged"], true);
    assert_eq!(provenance["verified"], true);
    assert_eq!(provenance["pid"], observed["pid"]);
    assert_eq!(provenance["pid"], summary["harness_pid"]);
    assert_eq!(provenance["process_started_at_utc"], observed["started_at_utc"]);
    assert_eq!(provenance["process_started_at_utc"], summary["harness_started_at_utc"]);
    assert_eq!(provenance["process_start_ticks"], observed["start_ticks"]);
    assert!(same_path(provenance["actual_process_image_path"].as_str().unwrap(), observed["image_path"].as_str().unwrap()));
    assert!(same_path(provenance["actual_process_image_path"].as_str().unwrap(), provenance["canonical_path"].as_str().unwrap()));
    assert_eq!(fs::read(temp.path().join(".codex_discord_bot.disabled")).unwrap(), MARKER);
}

#[test]
#[rustfmt::skip]
fn prov_02_wrong_expected_hash_fails_closed_before_child_start() {
    let temp = tempfile::tempdir().unwrap();
    let (target, harness) = prepare(temp.path());
    let actual_hash = sha256(&harness);
    let wrong_hash = "ab".repeat(32);
    assert_ne!(actual_hash, wrong_hash.to_ascii_uppercase());
    let output = soak_command(temp.path(), &target, &harness, &wrong_hash, "1").output().unwrap();
    assert!(!output.status.success());

    let summary = wrapper_summary(temp.path());
    let provenance = &summary["harness_provenance"];
    assert!(provenance.is_object());
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3"); assert!(summary.get("status").is_none());
    assert_eq!(summary["operational_status"], "integrity_failed"); assert_eq!(summary["final_eligibility"]["status"], "failed"); assert_eq!(summary["final_eligibility"]["eligible"], false);
    let failure = &summary["source_provenance"]["primary_failure"];
    assert_eq!(failure["stage"], "harness_hash_verification"); assert_eq!(failure["code"], "expected_hash_mismatch"); assert_eq!(failure["message"], "ExpectedHarnessSha256 does not match the opened offline soak harness");
    assert!(summary["harness_pid"].is_null());
    assert!(summary["harness_started_at_utc"].is_null());
    assert!(summary["child_exit_code"].is_null());
    assert!(same_path(provenance["canonical_path"].as_str().unwrap(), &fs::canonicalize(&harness).unwrap().to_string_lossy()));
    assert_eq!(provenance["expected_sha256"], wrong_hash.to_ascii_uppercase());
    assert_eq!(provenance["sha256_before"], actual_hash);
    assert_eq!(provenance["length_before_bytes"], fs::metadata(&harness).unwrap().len());
    for field in ["sha256_after", "length_after_bytes", "unchanged", "pid", "process_started_at_utc",
        "process_start_ticks", "actual_process_image_path"] {
        assert!(provenance[field].is_null(), "{field} must be null before launch");
    }
    assert_eq!(provenance["verified"], false);
    assert!(!fs::read_dir(temp.path().join("evidence")).unwrap().any(|entry|
        entry.unwrap().path().to_string_lossy().ends_with(".memory.jsonl")));
    assert_eq!(fs::read(temp.path().join(".codex_discord_bot.disabled")).unwrap(), MARKER);
}

#[test]
#[rustfmt::skip]
fn prov_03_changed_harness_cannot_produce_passed_evidence() {
    const CHANGED: &[u8] = b"changed-after-launch";
    let temp = tempfile::tempdir().unwrap();
    let (target, harness) = prepare(temp.path());
    let expected_hash = sha256(&harness);
    let expected_len = fs::metadata(&harness).unwrap().len();
    let changed_hash = hex::encode_upper(Sha256::digest(CHANGED));
    let mut command = soak_command(temp.path(), &target, &harness, &expected_hash, "3");
    let child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let (child, _) = wait_for_memory(child, temp.path());
    let replacement = fs::write(&harness, CHANGED);
    let output = wait_for_output(child, Duration::from_secs(12));
    let summary = wrapper_summary(temp.path());
    let provenance = &summary["harness_provenance"];

    match replacement {
        Ok(()) => {
            assert!(!output.status.success());
            assert_eq!(summary["operational_status"], "integrity_failed"); assert_eq!(summary["final_eligibility"]["status"], "failed"); assert_eq!(summary["final_eligibility"]["eligible"], false);
            let failure = &summary["source_provenance"]["primary_failure"]; assert_eq!(failure["stage"], "harness_hash_verification"); assert_eq!(failure["code"], "harness_changed_after_launch"); assert_eq!(failure["message"], "Offline soak harness provenance changed after launch");
            assert_eq!(provenance["expected_sha256"], expected_hash);
            assert_eq!(provenance["sha256_before"], expected_hash);
            assert_eq!(provenance["length_before_bytes"], expected_len);
            assert_eq!(provenance["sha256_after"], changed_hash);
            assert_eq!(provenance["length_after_bytes"], CHANGED.len() as u64);
            assert_eq!(provenance["unchanged"], false);
            assert_eq!(provenance["verified"], false);
        }
        Err(error) => {
            assert!(matches!(error.raw_os_error(), Some(32 | 33)),
                "replacement refusal must be a Windows sharing/lock violation: {error}");
            assert_eq!(sha256(&harness), expected_hash, "refused replacement changed bytes");
            assert_success(&output);
            assert_eq!(summary["operational_status"], "passed"); assert_eq!(summary["final_eligibility"]["status"], "ineligible"); assert_eq!(summary["final_eligibility"]["eligible"], false);
            assert_eq!(provenance["unchanged"], true);
            assert_eq!(provenance["verified"], true);
        }
    }
    assert_eq!(fs::read(temp.path().join(".codex_discord_bot.disabled")).unwrap(), MARKER);
}
