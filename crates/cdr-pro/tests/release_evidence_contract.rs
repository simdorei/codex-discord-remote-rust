use cdr_pro::release_evidence::{Check, DEFERRED, Evidence, REQUIRED, Status};
fn complete() -> Vec<Check> {
    REQUIRED
        .iter()
        .map(|id| Check::new(id, "source", Status::Passed))
        .collect()
}
fn evidence(checks: Vec<Check>) -> Evidence {
    Evidence::new(
        "revision-1".into(),
        "dirty".into(),
        "windows".into(),
        "1.2.3".into(),
        checks,
    )
}

#[test]
fn evidence_is_deterministic_ordered_public_safe_and_never_release_ready() {
    let mut checks = complete();
    checks.reverse();
    let evidence = evidence(checks);
    let root = tempfile::tempdir().unwrap();
    evidence.write(&root.path().join("first.json")).unwrap();
    evidence.write(&root.path().join("second.json")).unwrap();
    let first = std::fs::read_to_string(root.path().join("first.json")).unwrap();
    assert_eq!(
        first,
        std::fs::read_to_string(root.path().join("second.json")).unwrap()
    );
    let payload: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(payload["pre_restart_ready"], true);
    assert_eq!(payload["release_ready"], false);
    assert_eq!(payload["deferred_check_ids"], serde_json::json!(DEFERRED));
    assert_eq!(
        evidence
            .checks
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        REQUIRED
    );
    for forbidden in [
        "source.path",
        "fingerprint",
        "project_scope",
        "conversation_scope",
        "token",
        "C:/private",
    ] {
        assert!(!first.contains(forbidden));
    }
}

#[test]
fn every_nonpassing_required_check_fails_closed() {
    for status in [
        Status::Failed,
        Status::Skipped,
        Status::Stale,
        Status::Malformed,
    ] {
        let mut checks = complete();
        checks[3].status = status;
        assert!(!evidence(checks).pre_restart_ready);
    }
}

#[test]
fn missing_duplicate_and_unexpected_checks_are_not_ready() {
    let mut duplicate = complete();
    duplicate.push(duplicate[0].clone());
    let mut unexpected = complete();
    unexpected.push(Check::new("unexpected", "source", Status::Passed));
    for checks in [complete()[1..].to_vec(), duplicate, unexpected, vec![]] {
        assert!(!evidence(checks).pre_restart_ready);
    }
}
