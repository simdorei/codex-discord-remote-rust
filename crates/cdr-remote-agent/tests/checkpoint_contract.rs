use std::fs;

use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_agent::files::ProjectFileAccess;
use cdr_remote_protocol::file_change::{FileChange, FileChangeAction};
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::output::{CoreOutput, ProjectOperationOutput};
use cdr_remote_protocol::request::{CoreRequest, ProjectOperation};
use chrono::{Duration, Utc};

const SESSION: &str = "session-aaaaaaaa";

async fn bound(root: &std::path::Path) -> LocalProjectDispatcher {
    let dispatcher = LocalProjectDispatcher::new();
    dispatcher
        .upsert("thread-a", root, Utc::now() + Duration::minutes(10))
        .await
        .expect("binding");
    let activated = dispatcher
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
    assert!(matches!(
        activated,
        BridgeResult::ProjectSessionResult { .. }
    ));
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

fn core(result: BridgeResult) -> CoreOutput {
    match result {
        BridgeResult::ProjectOperationResult {
            output: ProjectOperationOutput::Core(output),
            ..
        } => output,
        unexpected => panic!("unexpected result: {unexpected:?}"),
    }
}

#[tokio::test]
async fn cp1_create_lists_checkpoint_and_restore_removes_the_created_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    let dispatcher = bound(directory.path()).await;
    let created = core(
        dispatcher
            .execute(
                operation(
                    "create",
                    CoreRequest::FileCreate {
                        path: "note.txt".into(),
                        content: "new".into(),
                        overwrite: false,
                    },
                ),
                None,
            )
            .await,
    );
    let CoreOutput::FileCreate { checkpoint_id, .. } = created else {
        panic!("file create output");
    };
    let listed = core(
        dispatcher
            .execute(operation("list", CoreRequest::CheckpointList), None)
            .await,
    );
    assert!(matches!(
        listed,
        CoreOutput::CheckpointList { ref checkpoints }
            if checkpoints.len() == 1 && checkpoints[0].checkpoint_id == checkpoint_id
    ));
    let restored = core(
        dispatcher
            .execute(
                operation(
                    "restore",
                    CoreRequest::CheckpointRestore {
                        checkpoint_id: checkpoint_id.clone(),
                    },
                ),
                None,
            )
            .await,
    );
    assert!(matches!(
        restored,
        CoreOutput::CheckpointRestore { ref restored_files, .. }
            if restored_files == &["note.txt"]
    ));
    assert!(!directory.path().join("note.txt").exists());
}

#[tokio::test]
async fn cp2_restore_rejects_a_later_writer_and_legacy_write_has_undo() {
    let directory = tempfile::tempdir().expect("tempdir");
    let dispatcher = bound(directory.path()).await;
    let written = dispatcher
        .execute(
            GatewayCommand::WriteFile {
                request_id: "legacy".into(),
                thread_id: "thread-a".into(),
                deadline_at: Utc::now() + Duration::minutes(1),
                computer_session_id: Some(SESSION.into()),
                path: "note.txt".into(),
                content: "owned".into(),
                expected_sha256: None,
            },
            None,
        )
        .await;
    assert!(matches!(written, BridgeResult::WriteFileResult { .. }));
    let checkpoints = fs::read_dir(directory.path().join(".codex-remote-mcp/checkpoints"))
        .expect("checkpoint directory")
        .count();
    assert_eq!(checkpoints, 1);
    let CoreOutput::CheckpointList { checkpoints } = core(
        dispatcher
            .execute(operation("list", CoreRequest::CheckpointList), None)
            .await,
    ) else {
        panic!("checkpoint list");
    };
    fs::write(directory.path().join("note.txt"), "concurrent").expect("later writer");
    let rejected = dispatcher
        .execute(
            operation(
                "restore",
                CoreRequest::CheckpointRestore {
                    checkpoint_id: checkpoints[0].checkpoint_id.clone(),
                },
            ),
            None,
        )
        .await;
    assert!(matches!(
        rejected,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "fileconflict"
    ));
    assert_eq!(
        fs::read_to_string(directory.path().join("note.txt")).unwrap(),
        "concurrent"
    );
}

#[tokio::test]
async fn cp3_multi_file_patch_is_atomic_and_one_checkpoint_restores_all_paths() {
    let directory = tempfile::tempdir().expect("tempdir");
    fs::write(directory.path().join("old.txt"), "first\nsecond\n").expect("fixture");
    let hash = ProjectFileAccess::open(directory.path())
        .expect("access")
        .read_file("old.txt", 1, 20)
        .expect("read")
        .sha256;
    let dispatcher = bound(directory.path()).await;
    let patched = core(
        dispatcher
            .execute(
                operation(
                    "patch",
                    CoreRequest::FileApplyPatch {
                        changes: vec![
                            FileChange {
                                action: FileChangeAction::Update,
                                path: "old.txt".into(),
                                content: Some("changed\n".into()),
                                destination: None,
                                expected_sha256: Some(hash),
                            },
                            FileChange {
                                action: FileChangeAction::Create,
                                path: "new.txt".into(),
                                content: Some("created\n".into()),
                                destination: None,
                                expected_sha256: None,
                            },
                        ],
                    },
                ),
                None,
            )
            .await,
    );
    let CoreOutput::FileApplyPatch {
        applied,
        checkpoint_id,
    } = patched
    else {
        panic!("patch output");
    };
    assert_eq!(applied.len(), 2);
    assert_eq!(
        fs::read_to_string(directory.path().join("old.txt")).unwrap(),
        "changed\n"
    );
    assert!(directory.path().join("new.txt").exists());
    let _ = core(
        dispatcher
            .execute(
                operation("restore", CoreRequest::CheckpointRestore { checkpoint_id }),
                None,
            )
            .await,
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("old.txt")).unwrap(),
        "first\nsecond\n"
    );
    assert!(!directory.path().join("new.txt").exists());
}
