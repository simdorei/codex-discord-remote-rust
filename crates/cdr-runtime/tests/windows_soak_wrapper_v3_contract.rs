#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod windows_soak_wrapper;

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{Value, json};
use windows_soak_wrapper as wrapper;

const FROZEN_CHECKS: [&str; 19] = [
    "same_run_canonical_release_build",
    "canonical_release_harness",
    "build_fingerprints_equal",
    "soak_fingerprints_equal",
    "harness_provenance_verified",
    "harness_process_exit_confirmed",
    "child_exit_zero",
    "harness_contract_passed",
    "event_stream_valid",
    "minimum_duration_met",
    "harness_duration_matches_request",
    "harness_elapsed_reached_request",
    "canonical_warmup",
    "sampling_interval_not_weakened",
    "slope_limit_not_weakened",
    "regression_samples_met",
    "memory_slope_passed",
    "disabled_marker_preserved",
    "cleanup_clean",
];

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("contract value must be an object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn expected_keys<'a>(names: &'a [&'a str]) -> BTreeSet<&'a str> {
    names.iter().copied().collect()
}

fn comparable_path(value: &str) -> String {
    value
        .strip_prefix(r"\\?\")
        .unwrap_or(value)
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn is_upper_sha256(value: &Value) -> bool {
    value.as_str().is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    })
}

fn assert_complete_fingerprint(value: &Value, fixture: &wrapper::Fixture) {
    assert_eq!(
        keys(value),
        expected_keys(&[
            "schema",
            "repo_root",
            "aggregate_sha256",
            "file_count",
            "total_bytes",
            "files",
        ])
    );
    assert_eq!(value["schema"], "cdr.rust-source-fingerprint.v1");
    assert!(is_upper_sha256(&value["aggregate_sha256"]));
    assert_eq!(
        comparable_path(value["repo_root"].as_str().expect("repo_root string")),
        comparable_path(&fixture.root.to_string_lossy())
    );

    let rows = value["files"]
        .as_array()
        .expect("complete fingerprint rows");
    assert!(!rows.is_empty());
    assert_eq!(value["file_count"].as_u64(), Some(rows.len() as u64));
    assert_eq!(
        value["total_bytes"].as_u64(),
        Some(
            rows.iter()
                .map(|row| row["bytes"].as_u64().expect("row byte count"))
                .sum()
        )
    );
    for row in rows {
        assert_eq!(keys(row), expected_keys(&["path", "bytes", "sha256"]));
        assert!(row["path"].as_str().is_some_and(|path| !path.is_empty()));
        assert!(row["bytes"].is_u64());
        assert!(is_upper_sha256(&row["sha256"]));
    }

    let source_row = rows
        .iter()
        .find(|row| row["path"] == "crates/probe/src/lib.rs")
        .expect("nested source fingerprint row must survive summary JSON depth 20");
    assert_eq!(source_row["bytes"], 5);
    assert_eq!(source_row["sha256"], wrapper::sha256(&fixture.source_file));
}

#[test]
fn w3_01_02_11_12_skip_build_summary_emits_the_connected_v3_contract() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let output = wrapper::wait_for_output(
        wrapper::skip_build_command(&fixture, 1).spawn().unwrap(),
        Duration::from_secs(15),
    );
    assert!(
        output.status.success(),
        "1-second SkipBuild fixture failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    wrapper::assert_marker(&fixture);

    let summary = wrapper::summary(&fixture);
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3");
    assert!(
        summary.get("status").is_none(),
        "v3 removes top-level status"
    );
    assert_eq!(summary["operational_status"], "passed");

    let provenance = &summary["source_provenance"];
    assert_eq!(
        keys(provenance),
        expected_keys(&[
            "schema",
            "build_before",
            "build_after",
            "soak_after",
            "build_fingerprints_equal",
            "soak_fingerprints_equal",
            "primary_failure",
            "secondary_failures",
        ])
    );
    assert_eq!(
        provenance["schema"],
        "cdr.windows-soak.source-provenance.v1"
    );
    for name in ["build_before", "build_after", "soak_after"] {
        assert_complete_fingerprint(&provenance[name], &fixture);
    }
    assert_eq!(provenance["build_before"], provenance["build_after"]);
    assert_eq!(provenance["build_before"], provenance["soak_after"]);
    assert_eq!(provenance["build_fingerprints_equal"], true);
    assert_eq!(provenance["soak_fingerprints_equal"], true);
    assert!(provenance["primary_failure"].is_null());
    assert_eq!(provenance["secondary_failures"], json!([]));

    let eligibility = &summary["final_eligibility"];
    assert_eq!(
        eligibility["schema"],
        "cdr.windows-soak.final-eligibility.v1"
    );
    assert_eq!(eligibility["status"], "ineligible");
    assert_eq!(eligibility["eligible"], false);
    let checks = &eligibility["checks"];
    let expected: Vec<_> = std::iter::once("operational_passed")
        .chain(FROZEN_CHECKS)
        .collect();
    assert_eq!(keys(checks), expected_keys(&expected));
    assert_eq!(checks.as_object().unwrap().len(), 20);
    assert_eq!(checks["operational_passed"], true);
    assert_eq!(checks["same_run_canonical_release_build"], false);
    assert_eq!(checks["minimum_duration_met"], false);
}
