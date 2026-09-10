#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use serde_json::Value;
use std::fs;
use std::process::{Command, Output};
use std::time::Duration;

fn run(mut command: Command) -> Output {
    let child = command.spawn().unwrap();
    wrapper::wait_for_output(child, Duration::from_secs(20))
}
fn assert_failure_shape(failure: &Value, stage: Option<&str>, code: Option<&str>) {
    let object = failure.as_object().expect("failure object");
    assert_eq!(object.len(), 3, "only stage, code, and message");
    for key in ["stage", "code", "message"] {
        assert!(object.contains_key(key), "missing failure field {key}");
    }
    let nonempty = |key| failure[key].as_str().is_some_and(|value| !value.is_empty());
    if let Some(expected) = stage {
        assert_eq!(failure["stage"], expected);
    } else {
        assert!(nonempty("stage"));
    }
    if let Some(expected) = code {
        assert_eq!(failure["code"], expected);
    } else {
        assert!(nonempty("code"));
    }
    assert!(nonempty("message"));
}

#[rustfmt::skip]
fn assert_failed_envelope(output: &Output, summary: &Value, operational_status: &str,
    primary: (Option<&str>, Option<&str>), secondary: &[(&str, &str)]) {
    assert!(
        !output.status.success(),
        "wrapper unexpectedly passed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3");
    assert!(
        summary.get("status").is_none(),
        "v3 removes top-level status"
    );
    assert_eq!(summary["operational_status"], operational_status);
    assert_eq!(summary["final_eligibility"]["status"], "failed");
    assert_eq!(summary["final_eligibility"]["eligible"], false);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("soak_passed"));

    let provenance = summary["source_provenance"]
        .as_object()
        .expect("v3 source provenance");
    assert_failure_shape(&provenance["primary_failure"], primary.0, primary.1);
    let actual = provenance["secondary_failures"]
        .as_array()
        .expect("chronological secondary failure ledger");
    assert_eq!(actual.len(), secondary.len());
    for (failure, (stage, code)) in actual.iter().zip(secondary) {
        assert_failure_shape(failure, Some(stage), Some(code));
    }
}

fn assert_before_child(summary: &Value) {
    assert!(summary["harness_pid"].is_null());
    assert!(summary["harness_started_at_utc"].is_null());
    assert!(summary["child_exit_code"].is_null());
    assert!(summary["harness_provenance"]["pid"].is_null());
}

fn assert_after_exact_child_exit(summary: &Value, exit_code: Option<i64>) {
    let provenance = &summary["harness_provenance"];
    assert_eq!(provenance["process_exit_confirmed"], true);
    assert_eq!(provenance["verified"], true);
    assert_eq!(provenance["pid"], summary["harness_pid"]);
    assert_eq!(
        provenance["process_started_at_utc"],
        summary["harness_started_at_utc"]
    );
    assert!(provenance["process_start_ticks"].as_i64().is_some());
    if let Some(expected) = exit_code {
        assert_eq!(summary["child_exit_code"], expected);
    } else {
        assert_ne!(summary["child_exit_code"].as_i64(), Some(0));
    }
}

fn mutate_source(fixture: &wrapper::Fixture) {
    assert_eq!(fs::read(&fixture.source_file).unwrap(), b"alpha");
    fs::write(&fixture.source_file, b"bravo").unwrap();
}

fn inject_owner_close_failure(fixture: &wrapper::Fixture) {
    let path = fixture
        .root
        .join("scripts/CodexDiscordSoak.ProcessOwnership.psm1");
    let mut module = fs::read_to_string(&path).unwrap();
    module.push_str(
        "\nfunction Close-CodexSoakProcessOwner { param([object]$Owner) \
         throw 'forced process ownership cleanup failure' }\n",
    );
    fs::write(path, module).unwrap();
}

#[test]
fn w3_03_successful_build_with_same_length_source_mutation_fails_before_child() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let cargo = wrapper::fake_cargo(&fixture, true, 0);
    let output = run(wrapper::build_command(&fixture, &cargo, 1));
    let summary = wrapper::summary(&fixture);

    assert_failed_envelope(
        &output,
        &summary,
        "integrity_failed",
        (
            Some("source_fingerprint_build_compare"),
            Some("source_changed_during_build"),
        ),
        &[],
    );
    assert_before_child(&summary);
    wrapper::assert_marker(&fixture);
}

#[test]
fn w3_04_nonzero_cargo_is_the_build_primary_failure() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let cargo = wrapper::fake_cargo(&fixture, false, 31);
    let output = run(wrapper::build_command(&fixture, &cargo, 1));
    let summary = wrapper::summary(&fixture);

    assert_failed_envelope(
        &output,
        &summary,
        "runtime_failed",
        (Some("build"), Some("build_failed")),
        &[],
    );
    assert_before_child(&summary);
    wrapper::assert_marker(&fixture);
}

#[test]
fn w3_05_build_failure_stays_primary_and_build_drift_is_secondary() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let cargo = wrapper::fake_cargo(&fixture, true, 31);
    let output = run(wrapper::build_command(&fixture, &cargo, 1));
    let summary = wrapper::summary(&fixture);

    assert_failed_envelope(
        &output,
        &summary,
        "runtime_failed",
        (Some("build"), Some("build_failed")),
        &[(
            "source_fingerprint_build_compare",
            "source_changed_during_build",
        )],
    );
    assert_before_child(&summary);
    wrapper::assert_marker(&fixture);
}

#[test]
fn w3_06_source_mutation_during_soak_fails_after_verified_child_exit() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let child = wrapper::skip_build_command(&fixture, 2).spawn().unwrap();
    let (child, _) = wrapper::wait_for_memory(child, &fixture);
    mutate_source(&fixture);
    let output = wrapper::wait_for_output(child, Duration::from_secs(12));
    let summary = wrapper::summary(&fixture);

    assert_failed_envelope(
        &output,
        &summary,
        "integrity_failed",
        (
            Some("source_fingerprint_soak_compare"),
            Some("source_changed_during_soak"),
        ),
        &[],
    );
    assert_after_exact_child_exit(&summary, Some(0));
    wrapper::assert_marker(&fixture);
}

#[test]
fn w3_07_runtime_failure_stays_primary_and_soak_drift_is_secondary() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let child = wrapper::skip_build_command(&fixture, 30).spawn().unwrap();
    let (child, memory) = wrapper::wait_for_memory(child, &fixture);
    mutate_source(&fixture);
    let pid = u32::try_from(memory["harness_pid"].as_u64().unwrap())
        .expect("harness PID must fit in a Windows u32 PID");
    wrapper::terminate_tree(pid);
    let output = wrapper::wait_for_output(child, Duration::from_secs(12));
    let summary = wrapper::summary(&fixture);

    assert_failed_envelope(
        &output,
        &summary,
        "runtime_failed",
        (None, None),
        &[(
            "source_fingerprint_soak_compare",
            "source_changed_during_soak",
        )],
    );
    let primary = &summary["source_provenance"]["primary_failure"];
    assert_ne!(primary["code"], "source_changed_during_soak");
    assert_ne!(primary["stage"], "source_fingerprint_soak_compare");
    assert_after_exact_child_exit(&summary, None);
    wrapper::assert_marker(&fixture);
}

#[test]
fn w3_08_cleanup_failure_is_primary_only_without_an_earlier_failure() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    inject_owner_close_failure(&fixture);
    let output = run(wrapper::skip_build_command(&fixture, 1));
    let summary = wrapper::summary(&fixture);

    assert_failed_envelope(
        &output,
        &summary,
        "runtime_failed",
        (Some("cleanup"), Some("cleanup_failed")),
        &[],
    );
    assert!(
        summary["source_provenance"]["primary_failure"]["message"]
            .as_str()
            .unwrap()
            .contains("forced process ownership cleanup failure")
    );
    assert_after_exact_child_exit(&summary, Some(0));
    wrapper::assert_marker(&fixture);
}
