use super::*;

#[test]
fn evidence_integrity_15_optional_absence_and_pending_live_allow_offline_checkpoint() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let before = read_json(&fixture.workspace_evidence);
    assert_eq!(
        before["native_tools"]["python_unavailable_execution"]["status"],
        "not_run"
    );
    assert_eq!(before["live_verification"]["status"], "pending");
    let output = run_checkpoint(&fixture, None, None, None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(read_json(&fixture.workspace_evidence), before);
    assert_eq!(fs::read(&fixture.marker).unwrap(), support::MARKER_BYTES);
}

#[test]
fn evidence_integrity_16_rejects_missing_incomplete_or_unobserved_operations() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let baseline = read_json(&fixture.workspace_evidence);
    for (field, invalid) in [
        ("status", json!("interrupted")),
        ("canary_observed", json!(false)),
        ("owned_process_starts", json!(0)),
        ("owned_process_starts", json!(2)),
        ("command_process", json!(null)),
        ("recognized_python_process_count", json!(1)),
        ("exit_code", json!(1)),
        ("command", json!("")),
        ("started_at", json!("2026-09-10T00:00:00Z")),
        ("ended_at", json!("2026-09-11T00:00:04Z")),
        ("live_network_invoked", json!(true)),
    ] {
        let mut record = baseline.clone();
        record["native_tools"]["process_observation"]["operations"][0][field] = invalid;
        write_json(&fixture.workspace_evidence, &record);
        assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");
    }
    for (field, invalid) in [
        ("observed", json!(false)),
        ("observed", json!("true")),
        ("pid", json!(0)),
        ("pid", json!("60")),
        ("pid", json!(4_294_967_296_u64)),
        ("name", json!("")),
        ("created_at", json!("2026-09-11T00:00:02.5Z")),
        ("exited_at", json!("2026-09-11T00:00:01.0Z")),
        ("start_event_at", json!("2026-09-11T00:00:00Z")),
        ("stop_event_at", json!("2026-09-11T00:00:04Z")),
    ] {
        let mut record = baseline.clone();
        record["native_tools"]["process_observation"]["operations"][0]["command_process"][field] =
            invalid;
        write_json(&fixture.workspace_evidence, &record);
        assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");
    }
    for invalid in [
        json!([]),
        json!(null),
        json!(vec![baseline["native_tools"]["process_observation"]["operations"][0].clone(); 5]),
    ] {
        let mut record = baseline.clone();
        record["native_tools"]["process_observation"]["operations"] = invalid;
        write_json(&fixture.workspace_evidence, &record);
        assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");
    }
}

#[test]
fn evidence_integrity_17_rejects_audit_absence_and_unscoped_live_approval() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let baseline = read_json(&fixture.workspace_evidence);
    for pointer in [
        "/native_tools/dependency_audit",
        "/native_tools/process_observation",
        "/live_verification",
    ] {
        let mut record = baseline.clone();
        *record.pointer_mut(pointer).unwrap() = json!(null);
        write_json(&fixture.workspace_evidence, &record);
        assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");
    }
    for (pointer, invalid) in [
        (
            "/native_tools/dependency_audit/rollback_source_sha256",
            json!("B".repeat(64)),
        ),
        (
            "/native_tools/process_observation/rollback_source_sha256",
            json!("B".repeat(64)),
        ),
        (
            "/native_tools/dependency_audit/source_fingerprint",
            json!("B".repeat(64)),
        ),
        ("/native_tools/dependency_audit/exit_code", json!(1)),
        ("/native_tools/dependency_audit/passed", json!(0)),
        (
            "/native_tools/python_unavailable_execution/required",
            json!(true),
        ),
        (
            "/native_tools/python_unavailable_execution/reason",
            json!(""),
        ),
        ("/live_verification/status", json!("passed")),
    ] {
        let mut record = baseline.clone();
        *record.pointer_mut(pointer).unwrap() = invalid;
        write_json(&fixture.workspace_evidence, &record);
        assert_contract_rejected_in_both_shells(&fixture.workspace_evidence, "workspace");
    }
}

#[test]
fn evidence_integrity_18_quality_counts_and_unreviewed_exceptions_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let baseline = read_json(&fixture.workspace_evidence);
    for field in [
        "rust_invalid_utf8_count",
        "text_invalid_utf8_count",
        "text_utf8_bom_count",
        "production_rust_files_over_250_lines",
    ] {
        let mut record = baseline.clone();
        record["quality"][field] = json!(1);
        write_json(&fixture.workspace_evidence, &record);
        let output = run_evidence_bundle_in_shell(&fixture, "powershell.exe");
        assert_rejected(&output, "quality");
    }
    let mut unreviewed = baseline;
    unreviewed["quality"]["exceptions"] = json!([{
        "path": "crates/fixture/src/lib.rs", "bom": false, "lines": 251,
        "reason": "self-approved by the submitted report"
    }]);
    write_json(&fixture.workspace_evidence, &unreviewed);
    let output = run_evidence_bundle_in_shell(&fixture, "powershell.exe");
    assert_rejected(&output, "quality");
}

#[test]
fn evidence_integrity_19_exact_reviewed_exception_passes_but_substitution_and_bad_utf8_fail() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = create_fixture(&temp.path().join("repo"));
    let path = "crates/fixture/src/lib.rs";
    fs::write(
        fixture.root.join(path),
        "// one cohesive fixture\n".repeat(251),
    )
    .unwrap();
    let approved =
        json!({"path":path,"bom":false,"lines":251,"reason":"One reviewed test responsibility."});
    write_json(
        &fixture
            .root
            .join("scripts/RustMigrationCheckpoint.QualityApprovals.json"),
        &json!({"schema":"cdr.reviewed-quality-exceptions.v1","exceptions":[approved.clone()]}),
    );
    support::refresh_fixture_evidence(&fixture);
    let mut baseline = read_json(&fixture.workspace_evidence);
    baseline["quality"]["production_rust_files_over_250_lines"] = json!(1);
    baseline["quality"]["exceptions"] = json!([approved.clone()]);
    write_json(&fixture.workspace_evidence, &baseline);
    for shell in ["powershell.exe", "pwsh.exe"] {
        let output = run_evidence_bundle_in_shell(&fixture, shell);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for (field, invalid) in [
        ("path", json!("crates/fixture/src/other.rs")),
        ("lines", json!(252)),
        ("bom", json!(true)),
        (
            "reason",
            json!("Report self-approval is not the reviewed policy."),
        ),
    ] {
        let mut record = baseline.clone();
        record["quality"]["exceptions"][0][field] = invalid;
        write_json(&fixture.workspace_evidence, &record);
        assert_rejected(
            &run_evidence_bundle_in_shell(&fixture, "powershell.exe"),
            "quality",
        );
    }
    for invalid in [json!([]), json!([approved.clone(), approved])] {
        let mut record = baseline.clone();
        record["quality"]["exceptions"] = invalid;
        write_json(&fixture.workspace_evidence, &record);
        assert_rejected(
            &run_evidence_bundle_in_shell(&fixture, "powershell.exe"),
            "quality",
        );
    }
    fs::write(fixture.root.join(path), [0xff, 0xfe, 0x00]).unwrap();
    support::refresh_fixture_evidence(&fixture);
    assert_rejected(
        &run_evidence_bundle_in_shell(&fixture, "powershell.exe"),
        "quality",
    );
}
