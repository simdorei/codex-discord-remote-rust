#![cfg(windows)]

#[path = "support/windows_memory_ab.rs"]
mod support;

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use support::{SampleRequest, run_sample, spawn_owned_process_pair, spawn_renamed_benign_runtime};

#[test]
fn sampling_requires_authorization_production_duration_and_app_server() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join(".codex_discord_bot.disabled");
    fs::write(&marker, b"operator_disabled\n").unwrap();
    let pair = spawn_owned_process_pair(temp.path());

    let mut request = SampleRequest::valid(&pair);
    request.authorized = false;
    let unauthorized = run_sample(temp.path(), &request);
    assert!(!unauthorized.status.success());
    assert!(String::from_utf8_lossy(&unauthorized.stderr).contains("AuthorizedLiveMeasurement"));

    let mut request = SampleRequest::valid(&pair);
    request.short_override = false;
    let too_short = run_sample(temp.path(), &request);
    assert!(!too_short.status.success());
    assert!(String::from_utf8_lossy(&too_short.stderr).contains("at least 300 seconds"));

    let mut request = SampleRequest::valid(&pair);
    request.short_override = false;
    request.non_discord_override = false;
    request.duration_seconds = "300";
    request.app = None;
    let missing_app = run_sample(temp.path(), &request);
    assert!(!missing_app.status.success());
    assert!(String::from_utf8_lossy(&missing_app.stderr).contains("Production sampling requires"));
    assert_eq!(fs::read(&marker).unwrap(), b"operator_disabled\n");
    assert!(!temp.path().join("samples.jsonl").exists());
    assert!(!temp.path().join("summary.json").exists());
}

#[test]
fn short_test_sampling_emits_attributable_raw_and_summary_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join(".codex_discord_bot.disabled");
    fs::write(&marker, b"operator_disabled\n").unwrap();
    let pair = spawn_owned_process_pair(temp.path());
    let request = SampleRequest::valid(&pair);
    let output = run_sample(temp.path(), &request);
    assert!(
        output.status.success(),
        "sample failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&marker).unwrap(), b"operator_disabled\n");

    let records = fs::read_to_string(temp.path().join("samples.jsonl")).unwrap();
    let parsed: Vec<Value> = records
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        parsed.len() >= 6,
        "expected repeated bot/app/combined samples"
    );
    for kind in ["bot", "app-server", "combined"] {
        assert!(parsed.iter().any(|record| record["runtime_kind"] == kind));
    }
    for record in &parsed {
        assert_eq!(record["schema"], "cdr.memory-ab.raw-sample.v1");
        assert_eq!(record["phase"], "warm_idle");
        assert_eq!(record["workload_id"], "workload-42");
        assert_eq!(record["db_snapshot_id"], "db-copy-7");
        assert_eq!(record["codex_version"], "0.146.0-test");
        assert_eq!(record["bot_pid"], pair.parent.id());
        assert_eq!(record["app_server_pid"], pair.app_pid);
        assert_eq!(
            record["bot_executable_path"].as_str().unwrap(),
            pair.bot_executable_path.to_string_lossy()
        );
        assert!(record["measurement_id"].as_str().is_some());
        for metric in [
            "working_set_bytes",
            "private_memory_bytes",
            "cpu_one_core_percent",
            "cpu_machine_percent",
            "handle_count",
            "thread_count",
        ] {
            assert!(record[metric].is_number(), "{metric} must be numeric");
        }
    }

    let summary: Value =
        serde_json::from_slice(&fs::read(temp.path().join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["schema"], "cdr.memory-ab.phase-summary.v1");
    assert_eq!(summary["pairing"]["workload_id"], "workload-42");
    assert_eq!(summary["sampling"]["production_minimum_seconds"], 300);
    assert_eq!(summary["sampling"]["test_only_short_override"], true);
    assert_eq!(summary["sampling"]["test_only_non_discord_processes"], true);
    assert_eq!(summary["identities"]["bot"]["pid"], pair.parent.id());
    assert_eq!(summary["identities"]["app_server"]["pid"], pair.app_pid);
    assert_eq!(
        summary["identities"]["bot"]["executable_path"]
            .as_str()
            .unwrap(),
        pair.bot_executable_path.to_string_lossy()
    );
    assert_eq!(summary["measurement_id"], parsed[0]["measurement_id"]);
    assert_eq!(summary["readiness"]["ready_before_measurement"], true);
    assert!(summary["startup"]["duration_milliseconds"].is_number());
    for kind in ["bot", "app-server", "combined"] {
        let metrics = &summary["results"][kind]["metrics"];
        for metric in [
            "working_set_bytes",
            "private_memory_bytes",
            "cpu_one_core_percent",
            "cpu_machine_percent",
            "handle_count",
            "thread_count",
        ] {
            for statistic in ["mean", "median", "max"] {
                assert!(metrics[metric][statistic].is_number());
            }
        }
    }
}

#[test]
fn sampling_rejects_start_executable_and_parent_identity_mismatches() {
    let temp = tempfile::tempdir().unwrap();
    let pair = spawn_owned_process_pair(temp.path());
    let mut request = SampleRequest::valid(&pair);
    request.bot_started_at_utc = "2000-01-01T00:00:00.0000000Z";
    let wrong_start = run_sample(temp.path(), &request);
    assert!(!wrong_start.status.success());
    assert!(String::from_utf8_lossy(&wrong_start.stderr).contains("identity mismatch"));

    let wrong_executable = PathBuf::from(env!("SystemRoot")).join("System32/cmd.exe");
    let mut request = SampleRequest::valid(&pair);
    request.bot_executable_path = &wrong_executable;
    let wrong_path = run_sample(temp.path(), &request);
    assert!(!wrong_path.status.success());
    assert!(String::from_utf8_lossy(&wrong_path.stderr).contains("executable identity mismatch"));

    let other_root = tempfile::tempdir().unwrap();
    let other = spawn_owned_process_pair(other_root.path());
    let mut request = SampleRequest::valid(&pair);
    request.app = Some(&other);
    let wrong_parent = run_sample(temp.path(), &request);
    assert!(!wrong_parent.status.success());
    assert!(String::from_utf8_lossy(&wrong_parent.stderr).contains("parent mismatch"));
    assert!(!temp.path().join("samples.jsonl").exists());
    assert!(!temp.path().join("summary.json").exists());
}

#[test]
fn sampling_refuses_preexisting_raw_or_summary_evidence_files() {
    let temp = tempfile::tempdir().unwrap();
    let pair = spawn_owned_process_pair(temp.path());
    let request = SampleRequest::valid(&pair);
    let raw = temp.path().join("samples.jsonl");
    let summary = temp.path().join("summary.json");

    fs::write(&raw, b"raw-sentinel").unwrap();
    let raw_exists = run_sample(temp.path(), &request);
    assert!(!raw_exists.status.success());
    assert!(String::from_utf8_lossy(&raw_exists.stderr).contains("output path already exists"));
    assert_eq!(fs::read(&raw).unwrap(), b"raw-sentinel");
    assert!(!summary.exists());

    fs::remove_file(&raw).unwrap();
    fs::write(&summary, b"summary-sentinel").unwrap();
    let summary_exists = run_sample(temp.path(), &request);
    assert!(!summary_exists.status.success());
    assert!(String::from_utf8_lossy(&summary_exists.stderr).contains("output path already exists"));
    assert_eq!(fs::read(&summary).unwrap(), b"summary-sentinel");
    assert!(!raw.exists());
}

#[test]
fn production_identity_scan_rejects_a_second_runtime_without_stopping_it() {
    let temp = tempfile::tempdir().unwrap();
    let pair = spawn_owned_process_pair(temp.path());
    let mut duplicate = spawn_renamed_benign_runtime(temp.path(), &pair.bot_executable_path);
    let mut request = SampleRequest::valid(&pair);
    request.non_discord_override = false;

    let output = run_sample(temp.path(), &request);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("second live Discord bot detected"));
    assert!(
        duplicate.is_running(),
        "measurement script must not stop the duplicate"
    );
    assert!(!temp.path().join("samples.jsonl").exists());
    assert!(!temp.path().join("summary.json").exists());
}
