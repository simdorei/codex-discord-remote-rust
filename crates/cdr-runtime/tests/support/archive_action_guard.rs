use super::*;
use cdr_store::ingress::{self, IngressKind, NewIngress};

#[tokio::test]
async fn archive_confirmation_preserves_a_target_with_an_unprocessed_request() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let db = Connection::open(&state).unwrap();
    db.execute_batch(include_str!("../fixtures/action_state.sql"))
        .unwrap();
    db.execute_batch(
        "CREATE TABLE thread_spawn_edges(parent_thread_id TEXT,child_thread_id TEXT);",
    )
    .unwrap();
    let archive = temp.path().join("archived_sessions");
    std::fs::create_dir(&archive).unwrap();
    let rollout = archive.join("old.jsonl");
    std::fs::write(&rollout, b"original transcript").unwrap();
    db.execute(
        "UPDATE threads SET rollout_path=? WHERE id='thread-old'",
        [rollout.to_string_lossy().as_ref()],
    )
    .unwrap();
    let mirror = temp.path().join("mirror.sqlite");
    ingress::admit(
        &mirror,
        &NewIngress {
            ingress_id: "message:1".into(),
            kind: IngressKind::Message,
            event_id: Some(1),
            application_id: None,
            channel_id: 99,
            owner_user_id: 20,
            source_message_id: Some(1),
            payload: serde_json::json!({"content":"pending"}),
            target_thread_id: Some("thread-old".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    let executor = ActionExecutor::new(
        state,
        mirror.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(
            mirror.clone(),
            Arc::new(ReadOnlyBackend),
        )),
    );
    let preview = executor
        .execute(
            CommandAction::DeleteArchivePreview {
                reference: "thread-old".into(),
            },
            99,
            20,
        )
        .await
        .unwrap();
    assert!(preview.text.contains("thread-old"));
    assert!(!temp.path().join("maintenance_backups").exists());
    let error = executor
        .execute(
            CommandAction::DeleteArchiveConfirm {
                reference: "thread-old".into(),
            },
            99,
            20,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ingress"), "{error}");
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE id='thread-old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(std::fs::read(&rollout).unwrap(), b"original transcript");
    assert_eq!(
        ingress::get(&mirror, "message:1").unwrap().unwrap().state,
        "staged"
    );
}

#[tokio::test]
async fn real_serialized_confirmation_can_finish_and_has_a_mirror_backup() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let db = Connection::open(&state).unwrap();
    db.execute_batch(include_str!("../fixtures/action_state.sql"))
        .unwrap();
    db.execute_batch(
        "CREATE TABLE thread_spawn_edges(parent_thread_id TEXT,child_thread_id TEXT);",
    )
    .unwrap();
    let archive = temp.path().join("archived_sessions");
    std::fs::create_dir(&archive).unwrap();
    let rollout = archive.join("old.jsonl");
    std::fs::write(&rollout, b"original transcript").unwrap();
    db.execute(
        "UPDATE threads SET rollout_path=? WHERE id='thread-old'",
        [rollout.to_string_lossy().as_ref()],
    )
    .unwrap();
    let mirror = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&mirror, "thread-old", "old", "Old title", 90, 99, 1.0)
        .unwrap();
    let action = CommandAction::DeleteArchiveConfirm {
        reference: "thread-old".into(),
    };
    let plan = cdr_runtime::message_plan::MessagePlan::Execute(action.clone());
    ingress::admit(
        &mirror,
        &NewIngress {
            ingress_id: "message:1".into(),
            kind: IngressKind::Message,
            event_id: Some(1),
            application_id: None,
            channel_id: 99,
            owner_user_id: 20,
            source_message_id: Some(1),
            payload: serde_json::json!({"plan":plan}),
            target_thread_id: Some("thread-old".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    ingress::begin_execution(&mirror, "message:1", "processing", None, 2.0).unwrap();
    let executor = ActionExecutor::new(
        state,
        mirror.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(
            mirror.clone(),
            Arc::new(ReadOnlyBackend),
        )),
    );
    let result = executor
        .execute_with_context(
            action,
            cdr_runtime::action_executor::ActionContext {
                channel_id: 99,
                user_id: 20,
                discord_message_id: Some(1),
                auto_queue_when_busy: false,
            },
        )
        .await
        .unwrap();
    assert!(result.text.contains("Archived thread deleted after backup"));
    let backup = result
        .text
        .lines()
        .find_map(|line| line.strip_prefix("backup_dir: "))
        .unwrap();
    let backup_db = Connection::open(std::path::Path::new(backup).join("mirror.sqlite")).unwrap();
    let mappings: i64 = backup_db
        .query_row(
            "SELECT COUNT(*) FROM mirror_threads WHERE codex_thread_id='thread-old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(mappings, 1);
    assert!(!rollout.exists());
    ingress::record_result(
        &mirror,
        "message:1",
        &serde_json::json!({"response":result.text}),
        3.0,
    )
    .unwrap();
    ingress::confirm(&mirror, "message:1", 4.0).unwrap();
    assert_eq!(
        ingress::get(&mirror, "message:1").unwrap().unwrap().state,
        "completed"
    );
}
