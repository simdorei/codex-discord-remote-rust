use cdr_store::ingress::{IngressKind, NewIngress};
use serde_json::json;

use super::*;

fn stage(database: &Path, key: &str) {
    cdr_store::ingress::admit(
        database,
        &NewIngress {
            ingress_id: key.into(),
            kind: IngressKind::Interaction,
            event_id: Some(41),
            application_id: Some(2),
            channel_id: 10,
            owner_user_id: 20,
            source_message_id: None,
            payload: json!({"work":"fixture"}),
            target_thread_id: None,
            canonical_owner: Some(key.into()),
            now: 1.0,
        },
    )
    .unwrap();
    assert!(cdr_store::ingress::acknowledge(database, key, 2.0).unwrap());
}

#[test]
fn cancellation_after_execution_claim_is_held_without_replay_authority() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("custody.sqlite");
    stage(&database, "interaction:41");

    drop(
        ExecutionCustody::begin(
            &database,
            &database,
            "interaction:41",
            InteractionProcessingMode::Execute,
        )
        .unwrap(),
    );

    let row = cdr_store::ingress::get(&database, "interaction:41")
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "held");
    assert_eq!(row.phase, "processing");
    assert_eq!(row.hold_reason, "interaction_processing_cancelled");
    assert!(!row.confirmation_delivered);
    assert!(
        ExecutionCustody::begin(
            &database,
            &database,
            "interaction:41",
            InteractionProcessingMode::Execute,
        )
        .is_err()
    );
}

#[test]
fn result_and_confirmation_are_two_durable_transitions() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("custody.sqlite");
    stage(&database, "interaction:41");
    let mut custody = ExecutionCustody::begin(
        &database,
        &database,
        "interaction:41",
        InteractionProcessingMode::Execute,
    )
    .unwrap();

    custody
        .record_result(&json!({"action_completed":true}))
        .unwrap();
    let recorded = cdr_store::ingress::get(&database, "interaction:41")
        .unwrap()
        .unwrap();
    assert_eq!(recorded.state, "completed");
    assert!(!recorded.confirmation_delivered);

    custody
        .finish_success(&json!({"should_not_replace":true}))
        .unwrap();
    let confirmed = cdr_store::ingress::get(&database, "interaction:41")
        .unwrap()
        .unwrap();
    assert_eq!(confirmed.outcome, Some(json!({"action_completed":true})));
    assert!(confirmed.confirmation_delivered);
}

#[test]
fn known_action_result_is_not_changed_to_unknown_when_confirmation_fails() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("custody.sqlite");
    stage(&database, "interaction:41");
    let mut action_runs = 0;
    let mut custody = ExecutionCustody::begin(
        &database,
        &database,
        "interaction:41",
        InteractionProcessingMode::Execute,
    )
    .unwrap();

    action_runs += 1;
    custody
        .record_result(&json!({"action_completed":true}))
        .unwrap();
    custody.hold_failed().unwrap();
    drop(custody);

    let row = cdr_store::ingress::get(&database, "interaction:41")
        .unwrap()
        .unwrap();
    assert_eq!(action_runs, 1);
    assert_eq!(row.state, "completed");
    assert_eq!(row.phase, "result_recorded");
    assert_eq!(row.hold_reason, "");
    assert!(!row.confirmation_delivered);
}

#[test]
fn same_ingress_key_in_a_different_database_cannot_authorize_execution() {
    let worker_root = tempfile::tempdir().unwrap();
    let custody_root = tempfile::tempdir().unwrap();
    let worker_database = worker_root.path().join("worker.sqlite");
    let custody_database = custody_root.path().join("custody.sqlite");
    stage(&worker_database, "interaction:41");
    stage(&custody_database, "interaction:41");

    let error = ExecutionCustody::begin(
        &worker_database,
        &custody_database,
        "interaction:41",
        InteractionProcessingMode::Execute,
    )
    .unwrap_err();

    assert!(error.to_string().contains("different worker database"));
    for database in [&worker_database, &custody_database] {
        let row = cdr_store::ingress::get(database, "interaction:41")
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "acknowledged");
    }
}
