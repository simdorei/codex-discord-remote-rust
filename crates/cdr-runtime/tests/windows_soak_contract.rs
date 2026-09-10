#![cfg(windows)]

use std::{fs, process::Command};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sha256(path: &std::path::Path) -> String {
    hex::encode_upper(Sha256::digest(fs::read(path).unwrap()))
}

#[rustfmt::skip]
fn wrapper_summary(root: &std::path::Path) -> Value {
    let path = fs::read_dir(root.join("target/soak")).unwrap().map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".summary.json")).expect("wrapper summary");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[rustfmt::skip]
fn assert_v3_failure(evidence: &Value, operational: &str, stage: &str, code: &str, message: &str) {
    assert_eq!(evidence["schema"], "cdr.windows-soak.summary.v3"); assert!(evidence.get("status").is_none());
    assert_eq!(evidence["operational_status"], operational); assert_eq!(evidence["final_eligibility"]["status"], "failed"); assert_eq!(evidence["final_eligibility"]["eligible"], false);
    let failure = &evidence["source_provenance"]["primary_failure"];
    assert_eq!(failure["stage"], stage); assert_eq!(failure["code"], code); assert_eq!(failure["message"], message);
}

#[test]
#[rustfmt::skip]
fn windows_soak_script_is_offline_pid_bound_and_defaults_to_one_day() {
    let text = fs::read_to_string(repo_root().join("codex-discord-rust-soak.ps1")).unwrap();
    let runtime_path = repo_root().join("scripts/CodexDiscordSoak.WrapperRuntime.psm1");
    let runtime = fs::read_to_string(runtime_path).unwrap();

    assert!(text.contains("$DurationSeconds = 86400"));
    assert!(text.contains("cdr-offline-soak.exe"));
    assert!(text.contains("--duration-secs"));
    assert!(text.contains("--events"));
    assert!(text.contains("CreateNoWindow = $true"));
    assert!(text.contains("CodexDiscordSoak.WrapperRuntime.psm1"));
    assert!(runtime.contains("function Add-CodexSoakMemorySample"));
    assert!(runtime.contains("$Process.Id -ne $ExpectedPid"));
    assert!(runtime.contains("$actualStart.Ticks -ne $ExpectedStartTicks"));
    assert!(runtime.contains("$Process.WorkingSet64"));
    assert!(runtime.contains("cdr.windows-soak.memory-sample.v1"));
    assert!(text.contains("slope_bytes_per_hour"));
    assert!(text.contains("[Diagnostics.Stopwatch]::StartNew()"));
    assert!(text.contains("Assert-CodexSoakDisabledMarker"));
    assert!(runtime.contains("function Assert-CodexSoakDisabledMarker"));
    assert!(text.contains("[IO.FileShare]::Read"));
    assert!(text.contains("[string]$ExpectedHarnessSha256"));
    assert!(text.contains("Harness must be the canonical cdr-offline-soak artifact"));
    assert!(text.contains("$startInfo.FileName = $harnessProvenance.canonical_path"));
}

#[test]
#[rustfmt::skip]
fn runtime_binary_is_rejected_before_launch_and_failure_evidence_is_written() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join(".codex_discord_bot.disabled");
    fs::write(&marker, b"operator_disabled\n").unwrap();
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo_root().join("codex-discord-rust-soak.ps1"))
        .arg("-RepoRoot")
        .arg(temp.path())
        .args(["-HarnessPath", env!("CARGO_BIN_EXE_cdr-runtime")])
        .args(["-SkipBuild", "-DurationSeconds", "1"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Harness must be the canonical cdr-offline-soak artifact")
    );
    assert_eq!(fs::read(&marker).unwrap(), b"operator_disabled\n");
    assert!(
        !temp
            .path()
            .join(".codex_discord_rust.runtime.lock")
            .exists()
    );
    let evidence = wrapper_summary(temp.path());
    assert_v3_failure(&evidence, "runtime_failed", "prepare", "prepare_failed",
        &format!("Harness must be the canonical cdr-offline-soak artifact: {}", env!("CARGO_BIN_EXE_cdr-runtime").replace('/', "\\")));
    assert!(evidence["harness_pid"].is_null());
    assert_eq!(evidence["disabled_marker"]["preserved"], false);

    let sentinel = temp.path().join("cdr-offline-soak.exe");
    fs::write(&sentinel, b"must-not-execute").unwrap();
    let renamed = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo_root().join("codex-discord-rust-soak.ps1"))
        .arg("-RepoRoot")
        .arg(temp.path())
        .arg("-HarnessPath")
        .arg(&sentinel)
        .args(["-SkipBuild", "-DurationSeconds", "1"])
        .output()
        .unwrap();
    assert!(!renamed.status.success());
    assert_eq!(fs::read(sentinel).unwrap(), b"must-not-execute");
}

#[test]
#[rustfmt::skip]
fn live_rust_lock_pid_blocks_the_offline_harness_before_launch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join(".codex_discord_bot.disabled"), b"operator_disabled\n").unwrap();
    fs::write(temp.path().join(".codex_discord_rust.runtime.lock"),
        format!("pid={}\n", std::process::id())).unwrap();
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo_root().join("codex-discord-rust-soak.ps1"))
        .arg("-RepoRoot")
        .arg(temp.path())
        .args(["-HarnessPath", env!("CARGO_BIN_EXE_cdr-offline-soak")])
        .args(["-SkipBuild", "-DurationSeconds", "1"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("Offline soak requires the Rust runtime stopped"));
    assert!(!temp.path().join("target/soak").read_dir().unwrap()
        .any(|entry| entry.unwrap().path().to_string_lossy().ends_with(".memory.jsonl")));
    let evidence = wrapper_summary(temp.path());
    assert_v3_failure(
        &evidence, "runtime_failed", "prepare", "prepare_failed",
        &format!(
            "Offline soak requires the Rust runtime stopped; live lock PID {}",
            std::process::id()
        ),
    );
}

#[test]
#[rustfmt::skip]
fn short_windows_wrapper_run_preserves_marker_and_emits_bound_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join(".codex_discord_bot.disabled");
    let marker_contents = b"operator_disabled\n";
    fs::write(&marker, marker_contents).unwrap();
    let script = repo_root().join("codex-discord-rust-soak.ps1");
    let harness = std::path::Path::new(env!("CARGO_BIN_EXE_cdr-offline-soak"));
    let expected_hash = sha256(harness);
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(["-RepoRoot"])
        .arg(temp.path())
        .arg("-HarnessPath")
        .arg(harness)
        .args(["-ExpectedHarnessSha256", &expected_hash])
        .args([
            "-SkipBuild",
            "-DurationSeconds",
            "1",
            "-SampleIntervalSeconds",
            "0.1",
            "-WarmupSeconds",
            "0",
            "-HarnessExitGraceSeconds",
            "5",
            "-MaxSlopeBytesPerHour",
            "1000000000000000",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "wrapper failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&marker).unwrap(), marker_contents);
    let soak_dir = temp.path().join("target/soak");
    let summary_path = fs::read_dir(&soak_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".summary.json")
                && !path.to_string_lossy().contains(".harness.summary.json")
        })
        .expect("wrapper summary");
    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3"); assert!(summary.get("status").is_none());
    assert_eq!(
        summary["harness_summary"]["schema"],
        "cdr.offline-soak.summary.v1"
    );
    assert_eq!(
        summary["harness_summary"]["assertions"]["offline_only"],
        true
    );
    assert_eq!(summary["operational_status"], "passed");
    assert_eq!(summary["final_eligibility"]["status"], "ineligible"); assert_eq!(summary["final_eligibility"]["eligible"], false);
    assert_eq!(summary["child_exit_code"], 0);
    assert_eq!(summary["disabled_marker"]["preserved"], true);
    assert_eq!(summary["memory"]["threshold_passed"], true);
    assert!(summary["memory"]["sample_count"].as_u64().unwrap() >= 2);
    assert!(summary["harness_pid"].as_u64().is_some());
    assert!(summary["harness_started_at_utc"].as_str().is_some());

    let memory_path = summary["outputs"]["memory_jsonl"].as_str().unwrap();
    let records = fs::read_to_string(memory_path).unwrap();
    assert!(records.lines().count() >= 2);
    for line in records.lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        assert_eq!(record["harness_pid"], summary["harness_pid"]);
        assert_eq!(
            record["harness_started_at_utc"],
            summary["harness_started_at_utc"]
        );
    }
}

#[test]
fn powershell_parser_accepts_the_soak_script() {
    let script = repo_root().join("codex-discord-rust-soak.ps1");
    let command = format!(
        "$e=$null; [void][Management.Automation.Language.Parser]::ParseFile('{}',[ref]$null,[ref]$e); if ($e.Count) {{ $e | % Message; exit 1 }}",
        script.display().to_string().replace('\'', "''")
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &command])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "parse failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
