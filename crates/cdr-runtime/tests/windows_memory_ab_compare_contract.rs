#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("codex-discord-memory-ab.ps1")
}

fn phase_summary(runtime: &str, workload: &str, mean: f64, startup_ms: f64) -> Value {
    fn stats(mean: f64) -> Value {
        json!({ "mean": mean, "median": mean * 0.9, "max": mean * 1.2 })
    }
    fn runtime_result(mean: f64) -> Value {
        json!({
            "sample_count": 300,
            "metrics": {
                "working_set_bytes": stats(mean),
                "private_memory_bytes": stats(mean + 10.0),
                "cpu_one_core_percent": stats(mean / 10.0),
                "cpu_machine_percent": stats(mean / 20.0),
                "handle_count": stats(mean + 30.0),
                "thread_count": stats(mean + 40.0)
            }
        })
    }
    json!({
        "schema": "cdr.memory-ab.phase-summary.v1",
        "status": "completed",
        "measurement_id": format!("measurement-{runtime}"),
        "runtime_label": runtime,
        "phase": "active",
        "pairing": {
            "workload_id": workload,
            "db_snapshot_id": "snapshot-A",
            "codex_version": "0.146.0"
        },
        "startup": { "duration_milliseconds": startup_ms },
        "readiness": { "ready_before_measurement": true },
        "sampling": {
            "requested_duration_seconds": 300.0,
            "actual_duration_seconds": 300.1,
            "sample_interval_seconds": 1.0,
            "test_only_short_override": false,
            "test_only_non_discord_processes": false
        },
        "identities": {
            "bot": {
                "pid": 101,
                "started_at_utc": "2026-08-31T00:00:00Z",
                "executable_path": "C:\\runtime\\bot.exe"
            },
            "app_server": {
                "pid": 102,
                "started_at_utc": "2026-08-31T00:00:01Z",
                "executable_path": "C:\\runtime\\app.exe",
                "parent_bot_pid": 101,
                "parent_verified": true
            }
        },
        "results": {
            "bot": runtime_result(mean),
            "app-server": runtime_result(mean / 2.0),
            "combined": runtime_result(mean * 1.5)
        }
    })
}

fn compare(baseline: &Path, candidate: &Path, output: &Path) -> std::process::Output {
    Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script_path())
        .arg("-BaselineSummaryPath")
        .arg(baseline)
        .arg("-CandidateSummaryPath")
        .arg(candidate)
        .arg("-ComparisonOutputPath")
        .arg(output)
        .output()
        .unwrap()
}

#[test]
fn compare_requires_matched_pairing_metadata_and_emits_numeric_deltas() {
    let temp = tempfile::tempdir().unwrap();
    let baseline = temp.path().join("baseline.json");
    let candidate = temp.path().join("candidate.json");
    let comparison = temp.path().join("comparison.json");
    fs::write(
        &baseline,
        serde_json::to_vec(&phase_summary("python", "same-work", 100.0, 1000.0)).unwrap(),
    )
    .unwrap();
    fs::write(
        &candidate,
        serde_json::to_vec(&phase_summary("rust", "same-work", 80.0, 700.0)).unwrap(),
    )
    .unwrap();

    let matched = compare(&baseline, &candidate, &comparison);
    assert!(
        matched.status.success(),
        "compare failed: {}",
        String::from_utf8_lossy(&matched.stderr)
    );
    let result: Value = serde_json::from_slice(&fs::read(&comparison).unwrap()).unwrap();
    assert_eq!(result["schema"], "cdr.memory-ab.comparison.v1");
    assert_eq!(result["pairing"]["workload_id"], "same-work");
    assert_eq!(
        result["metric_deltas"]["bot"]["working_set_bytes"]["mean"],
        -20.0
    );
    assert_eq!(result["startup_deltas"]["duration_milliseconds"], -300.0);
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains("threshold"));
    assert!(!serialized.contains("passed"));

    let original_comparison = fs::read(&comparison).unwrap();
    let output_exists = compare(&baseline, &candidate, &comparison);
    assert!(!output_exists.status.success());
    assert!(String::from_utf8_lossy(&output_exists.stderr).contains("output path already exists"));
    assert_eq!(fs::read(&comparison).unwrap(), original_comparison);

    for (field, different) in [
        ("workload_id", "different-work"),
        ("db_snapshot_id", "different-snapshot"),
        ("codex_version", "different-version"),
    ] {
        let mut mismatched = phase_summary("rust", "same-work", 80.0, 700.0);
        mismatched["pairing"][field] = Value::String(different.into());
        fs::write(&candidate, serde_json::to_vec(&mismatched).unwrap()).unwrap();
        if comparison.exists() {
            fs::remove_file(&comparison).unwrap();
        }
        let mismatch = compare(&baseline, &candidate, &comparison);
        assert!(!mismatch.status.success());
        assert!(String::from_utf8_lossy(&mismatch.stderr).contains(&format!("{field} mismatch")));
        assert!(!comparison.exists());
    }

    let mut incomplete = phase_summary("rust", "same-work", 80.0, 700.0);
    incomplete["results"]
        .as_object_mut()
        .unwrap()
        .remove("combined");
    fs::write(&candidate, serde_json::to_vec(&incomplete).unwrap()).unwrap();
    let rejected = compare(&baseline, &candidate, &comparison);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("shape mismatch"));

    let nonfinite = serde_json::to_string(&phase_summary("rust", "same-work", 80.0, 700.0))
        .unwrap()
        .replace("\"mean\":80.0", "\"mean\":NaN");
    fs::write(&candidate, nonfinite).unwrap();
    let rejected = compare(&baseline, &candidate, &comparison);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("not finite"));
    assert!(!comparison.exists());
}
