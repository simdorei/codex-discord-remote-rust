use std::fs;
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;

#[tokio::test]
async fn offline_soak_drains_state_and_proves_dedup_retry_contract() {
    let temp = tempfile::tempdir().expect("temporary soak directory");
    let summary_path = temp.path().join("summary.json");
    let events_path = temp.path().join("events.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_cdr-offline-soak"))
        .args([
            "--duration-secs",
            "2",
            "--seed",
            "424242",
            "--output",
            summary_path.to_str().expect("UTF-8 summary path"),
            "--events",
            events_path.to_str().expect("UTF-8 events path"),
        ])
        .env_clear()
        .current_dir(temp.path())
        .output();
    let output = tokio::time::timeout(Duration::from_secs(10), output)
        .await
        .expect("offline soak must terminate within ten seconds")
        .expect("offline soak binary runs");

    assert!(
        output.status.success(),
        "offline soak failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value =
        serde_json::from_slice(&fs::read(&summary_path).expect("summary JSON exists"))
            .expect("summary is valid JSON");
    assert_summary(&summary);
    assert_progress(&events_path, &summary);
}

fn assert_summary(summary: &Value) {
    assert_eq!(summary["schema"], "cdr.offline-soak.summary.v1");
    assert_eq!(summary["mode"], "offline_fake_replay");
    assert_eq!(summary["seed"], 424_242);
    assert_eq!(summary["duration_secs"], 2);
    assert_eq!(summary["status"], "passed");
    assert_eq!(summary["tracker_backend"], "sqlite");
    let cycles = summary["cycles"].as_u64().unwrap_or_default();
    // The binary contract is duration-bound, not a machine-speed benchmark.
    // Repetition and its fixed digest are checked by repeated_cycle_contract
    // with exactly two full harness cycles, regardless of scheduler delays.
    assert!(
        cycles >= 1,
        "duration-bound proof must execute a complete cycle"
    );

    let counters = &summary["counters"];
    assert_eq!(counters["queue_remaining"], 0);
    assert_eq!(counters["outbox_remaining"], 0);
    assert_eq!(counters["duplicate_successes"], 0);
    assert_eq!(counters["target_stalls"], 0);
    assert_eq!(counters["mirror_send_failures"], 1);
    assert_eq!(counters["mirror_send_retries"], 1);
    assert_eq!(counters["mirror_retry_same_message"], 1);
    assert_eq!(counters["queue_submitted"], counters["queue_completed"]);
    assert_eq!(counters["outbox_staged"], counters["outbox_delivered"]);
    assert_eq!(counters["queue_submitted"], cycles * 4 + 2);
    assert_eq!(counters["mirror_sent"], cycles);
    assert_eq!(counters["mirror_events"], cycles * 3);
    assert_eq!(counters["recovery_injections"], 1);
    assert_eq!(counters["recovery_attempts"], 3);
    assert_eq!(counters["recovery_successes"], 1);
    assert_eq!(counters["healthy_target_progressed"], 1);
    assert_eq!(counters["queue_accepted_with_warning"], 1);
    assert_eq!(counters["recovery_deferred_until_due"], 1);
    assert_eq!(counters["retry_clock_advances"], 1);
    assert_eq!(counters["generation_reuse_restarts"], 1);
    assert!(counters["tracker_cache_kib"].as_u64().unwrap_or_default() <= 256);
    assert_eq!(
        counters["tracker_disk_rows"],
        counters["outbox_delivered"].as_u64().unwrap() + counters["mirror_sent"].as_u64().unwrap()
    );
    assert_eq!(summary["schedule_version"], 1);
    let digest = summary["schedule_digest"].as_str().unwrap();
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));

    for assertion in [
        "offline_only",
        "queue_drained",
        "outbox_drained",
        "no_duplicate_success",
        "no_target_stall",
        "failed_send_retried",
        "deterministic_schedule_exercised",
        "healthy_target_progressed_while_peer_unavailable",
        "durable_queue_backoff_exercised",
        "generation_reuse_recovery_exercised",
        "repeated_assistant_shapes_deduped",
        "disk_backed_exact_tracker",
    ] {
        assert_eq!(summary["assertions"][assertion], true, "{assertion}");
    }
}

fn assert_progress(events_path: &std::path::Path, summary: &Value) {
    let events = fs::read_to_string(events_path).expect("progress JSONL exists");
    let records = events
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("valid progress JSON line"))
        .collect::<Vec<_>>();
    assert!(!records.is_empty(), "at least one progress record");
    assert!(records.len() <= 64, "progress output must stay bounded");
    for pair in records.windows(2) {
        let previous_elapsed = pair[0]["elapsed_ms"].as_u64().expect("elapsed integer");
        let next_elapsed = pair[1]["elapsed_ms"].as_u64().expect("elapsed integer");
        let previous_cycle = pair[0]["cycle"].as_u64().expect("cycle integer");
        let next_cycle = pair[1]["cycle"].as_u64().expect("cycle integer");
        assert!(previous_elapsed <= next_elapsed);
        assert!(previous_cycle <= next_cycle);
    }
    assert_eq!(records.last().unwrap()["status"], "passed");
    assert_eq!(records.last().unwrap()["counters"], summary["counters"]);
    for record in records {
        assert_eq!(record["schema"], "cdr.offline-soak.progress.v1");
        assert_eq!(record["mode"], "offline_fake_replay");
        assert_eq!(record["seed"], 424_242);
        assert!(record["cycle"].as_u64().is_some());
    }
}

#[tokio::test]
async fn three_core_args_derive_events_path_and_output_alias_is_rejected() {
    let temp = tempfile::tempdir().expect("temporary soak directory");
    let summary = temp.path().join("derived-summary.json");
    let output = Command::new(env!("CARGO_BIN_EXE_cdr-offline-soak"))
        .args([
            "--duration-secs",
            "1",
            "--seed",
            "7",
            "--output",
            summary.to_str().expect("UTF-8 summary path"),
        ])
        .env_clear()
        .current_dir(temp.path())
        .output();
    let output = tokio::time::timeout(Duration::from_secs(10), output)
        .await
        .expect("derived-path soak terminates")
        .expect("derived-path soak runs");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(summary.with_extension("events.jsonl").is_file());

    let collision = temp.path().join("collision.json");
    let collision_alias = temp.path().join("sub/../collision.json");
    let output = Command::new(env!("CARGO_BIN_EXE_cdr-offline-soak"))
        .args([
            "--duration-secs",
            "1",
            "--seed",
            "7",
            "--output",
            collision_alias.to_str().expect("UTF-8 collision alias"),
            "--events",
            collision.to_str().expect("UTF-8 collision path"),
        ])
        .env_clear()
        .current_dir(temp.path())
        .output()
        .await
        .expect("collision check runs");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must be different"));
    assert!(!collision.exists());
}

#[tokio::test]
async fn preexisting_evidence_is_preserved_and_no_peer_file_is_created() {
    let temp = tempfile::tempdir().unwrap();
    let summary = temp.path().join("summary.json");
    let events = temp.path().join("events.jsonl");
    fs::write(&summary, "do-not-overwrite").unwrap();
    let output = run_binary(
        temp.path(),
        &summary,
        &events,
        &["--duration-secs", "1", "--seed", "9"],
    )
    .await;

    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(summary).unwrap(), "do-not-overwrite");
    assert!(!events.exists());
}

#[tokio::test]
async fn mid_run_failure_writes_failed_summary_and_terminal_progress() {
    let temp = tempfile::tempdir().unwrap();
    let summary_path = temp.path().join("summary.json");
    let events_path = temp.path().join("events.jsonl");
    let output = run_binary(
        temp.path(),
        &summary_path,
        &events_path,
        &[
            "--duration-secs",
            "2",
            "--seed",
            "11",
            "--test-fail-after-cycles",
            "1",
        ],
    )
    .await;

    assert!(!output.status.success());
    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["status"], "failed");
    assert!(summary["failure_reason"].as_str().is_some());
    let records = fs::read_to_string(events_path).unwrap();
    let terminal: Value = serde_json::from_str(records.lines().last().unwrap()).unwrap();
    assert_eq!(terminal["status"], "failed");
    assert_eq!(terminal["counters"], summary["counters"]);
}

async fn run_binary(
    root: &std::path::Path,
    summary: &std::path::Path,
    events: &std::path::Path,
    leading_args: &[&str],
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cdr-offline-soak"));
    command.args(leading_args).args([
        "--output",
        summary.to_str().unwrap(),
        "--events",
        events.to_str().unwrap(),
    ]);
    tokio::time::timeout(
        Duration::from_secs(10),
        command.env_clear().current_dir(root).output(),
    )
    .await
    .expect("offline soak terminates")
    .expect("offline soak runs")
}
