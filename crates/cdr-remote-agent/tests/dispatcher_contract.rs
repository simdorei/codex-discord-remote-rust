use std::fs;

use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use chrono::{Duration, Utc};

const SESSION_A: &str = "session-aaaaaaaa";
const SESSION_B: &str = "session-bbbbbbbb";

fn session(request_id: &str, session_id: &str, generation: u64) -> GatewayCommand {
    GatewayCommand::ProjectSession {
        request_id: request_id.into(),
        thread_id: "thread-a".into(),
        deadline_at: Utc::now() + Duration::minutes(1),
        computer_session_id: session_id.into(),
        computer_session_generation: generation,
    }
}

fn read(request_id: &str, session_id: Option<&str>) -> GatewayCommand {
    GatewayCommand::ReadFile {
        request_id: request_id.into(),
        thread_id: "thread-a".into(),
        deadline_at: Utc::now() + Duration::minutes(1),
        computer_session_id: session_id.map(str::to_owned),
        path: "notes.txt".into(),
        start_line: 2,
        max_lines: 20,
    }
}

#[tokio::test]
async fn d1_bound_current_session_reads_and_missing_binding_fails_closed() {
    let dispatcher = LocalProjectDispatcher::new();
    let missing = dispatcher.execute(read("missing", None), Some(1)).await;
    assert!(matches!(
        missing,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "binding_missing"
    ));

    let directory = tempfile::tempdir().expect("tempdir");
    fs::write(directory.path().join("notes.txt"), "first\nsecond").expect("fixture");
    dispatcher.begin_connection(1).await.expect("connection");
    dispatcher
        .upsert(
            "thread-a",
            directory.path(),
            Utc::now() + Duration::minutes(10),
        )
        .await
        .expect("binding");
    assert!(matches!(
        dispatcher
            .execute(session("activate", SESSION_A, 1), Some(1))
            .await,
        BridgeResult::ProjectSessionResult { .. }
    ));
    let result = dispatcher
        .execute(read("read", Some(SESSION_A)), Some(1))
        .await;
    assert!(matches!(
        result,
        BridgeResult::ReadFileResult { ref output, .. } if output.content == "second"
    ));
}

#[tokio::test]
async fn d2_stale_connection_and_session_generations_are_rejected() {
    let directory = tempfile::tempdir().expect("tempdir");
    let dispatcher = LocalProjectDispatcher::new();
    dispatcher.begin_connection(2).await.expect("connection");
    dispatcher
        .upsert(
            "thread-a",
            directory.path(),
            Utc::now() + Duration::minutes(10),
        )
        .await
        .expect("binding");
    let stale_connection = dispatcher
        .execute(session("old", SESSION_A, 1), Some(1))
        .await;
    assert!(matches!(
        stale_connection,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "computer_control"
    ));
    assert!(matches!(
        dispatcher
            .execute(session("one", SESSION_A, 2), Some(2))
            .await,
        BridgeResult::ProjectSessionResult { .. }
    ));
    assert!(matches!(
        dispatcher
            .execute(session("two", SESSION_B, 3), Some(2))
            .await,
        BridgeResult::ProjectSessionResult { .. }
    ));
    let stale_session = dispatcher
        .execute(read("stale", Some(SESSION_A)), Some(2))
        .await;
    assert!(matches!(
        stale_session,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "computer_control"
    ));
    let old_generation = dispatcher
        .execute(session("older", SESSION_A, 2), Some(2))
        .await;
    assert!(matches!(
        old_generation,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "computer_control"
    ));
}

#[tokio::test]
async fn d3_expiry_deadline_and_mutation_idempotency_match_wire_contract() {
    let directory = tempfile::tempdir().expect("tempdir");
    let dispatcher = LocalProjectDispatcher::new();
    dispatcher
        .upsert(
            "thread-a",
            directory.path(),
            Utc::now() - Duration::seconds(1),
        )
        .await
        .expect("binding");
    let expired = dispatcher.execute(read("expired", None), None).await;
    assert!(matches!(
        expired,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "binding_expired"
    ));

    dispatcher
        .upsert(
            "thread-a",
            directory.path(),
            Utc::now() + Duration::minutes(10),
        )
        .await
        .expect("binding");
    assert!(matches!(
        dispatcher
            .execute(session("activate", SESSION_A, 1), None)
            .await,
        BridgeResult::ProjectSessionResult { .. }
    ));
    let command = GatewayCommand::WriteFile {
        request_id: "write-once".into(),
        thread_id: "thread-a".into(),
        deadline_at: Utc::now() + Duration::minutes(1),
        computer_session_id: Some(SESSION_A.into()),
        path: "created.txt".into(),
        content: "first".into(),
        expected_sha256: None,
    };
    let first = dispatcher.execute(command.clone(), None).await;
    let duplicate = dispatcher.execute(command, None).await;
    assert_eq!(duplicate, first);
    let conflict = dispatcher
        .execute(
            GatewayCommand::WriteFile {
                request_id: "write-once".into(),
                thread_id: "thread-a".into(),
                deadline_at: Utc::now() + Duration::minutes(1),
                computer_session_id: Some(SESSION_A.into()),
                path: "other.txt".into(),
                content: "second".into(),
                expected_sha256: None,
            },
            None,
        )
        .await;
    assert!(matches!(
        conflict,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "request_id_conflict"
    ));
    assert!(!directory.path().join("other.txt").exists());

    let deadline = dispatcher
        .execute(
            GatewayCommand::ProjectInfo {
                request_id: "late".into(),
                thread_id: "thread-a".into(),
                deadline_at: Utc::now() - Duration::seconds(1),
                computer_session_id: Some(SESSION_A.into()),
            },
            None,
        )
        .await;
    assert!(matches!(
        deadline,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "request_expired"
    ));
}
