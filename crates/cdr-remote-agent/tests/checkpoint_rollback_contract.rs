use std::fs;

use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_agent::files::ProjectFileAccess;
use cdr_remote_protocol::file_change::{FileChange, FileChangeAction};
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::request::{CoreRequest, ProjectOperation};
use chrono::{Duration, Utc};

const SESSION: &str = "session-aaaaaaaa";

async fn bound(root: &std::path::Path) -> LocalProjectDispatcher {
    let dispatcher = LocalProjectDispatcher::new();
    dispatcher
        .upsert("thread-a", root, Utc::now() + Duration::minutes(10))
        .await
        .expect("binding");
    let result = dispatcher
        .execute(
            GatewayCommand::ProjectSession {
                request_id: "activate".into(),
                thread_id: "thread-a".into(),
                deadline_at: Utc::now() + Duration::minutes(1),
                computer_session_id: SESSION.into(),
                computer_session_generation: 1,
            },
            None,
        )
        .await;
    assert!(matches!(result, BridgeResult::ProjectSessionResult { .. }));
    dispatcher
}

fn operation(request_id: &str, operation: CoreRequest) -> GatewayCommand {
    GatewayCommand::ProjectOperation {
        request_id: request_id.into(),
        thread_id: "thread-a".into(),
        deadline_at: Utc::now() + Duration::minutes(1),
        computer_session_id: Some(SESSION.into()),
        operation: ProjectOperation::Core(operation),
    }
}

#[tokio::test]
async fn cp4_partial_patch_and_checkpoint_persist_failures_roll_back_owned_writes() {
    let directory = tempfile::tempdir().expect("tempdir");
    fs::write(directory.path().join("same.txt"), "original\n").expect("fixture");
    let hash = ProjectFileAccess::open(directory.path())
        .expect("access")
        .read_file("same.txt", 1, 20)
        .expect("read")
        .sha256;
    let dispatcher = bound(directory.path()).await;
    let duplicate_path = vec![
        FileChange {
            action: FileChangeAction::Update,
            path: "same.txt".into(),
            content: Some("first mutation\n".into()),
            destination: None,
            expected_sha256: Some(hash.clone()),
        },
        FileChange {
            action: FileChangeAction::Update,
            path: "same.txt".into(),
            content: Some("must not land\n".into()),
            destination: None,
            expected_sha256: Some(hash),
        },
    ];
    let failed = dispatcher
        .execute(
            operation(
                "partial",
                CoreRequest::FileApplyPatch {
                    changes: duplicate_path,
                },
            ),
            None,
        )
        .await;
    assert!(matches!(failed, BridgeResult::OperationError { .. }));
    assert_eq!(
        fs::read_to_string(directory.path().join("same.txt")).unwrap(),
        "original\n"
    );

    fs::write(directory.path().join(".codex-remote-mcp"), "blocks store")
        .expect("block checkpoint directory");
    let failed = dispatcher
        .execute(
            operation(
                "persist-failure",
                CoreRequest::FileCreate {
                    path: "rolled-back.txt".into(),
                    content: "temporary".into(),
                    overwrite: false,
                },
            ),
            None,
        )
        .await;
    assert!(matches!(failed, BridgeResult::OperationError { .. }));
    assert!(!directory.path().join("rolled-back.txt").exists());
}
