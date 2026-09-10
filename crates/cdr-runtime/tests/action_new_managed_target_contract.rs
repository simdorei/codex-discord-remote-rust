use std::sync::Arc;

use cdr_runtime::action_executor::ActionContext;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_store::queue::{is_app_server_managed_target, list};

#[path = "support/action_app_server.rs"]
mod action_app_server;
#[path = "support/action_target.rs"]
mod action_target;
use action_app_server::{rpc_log, start_fake_server};
use action_target::{FakeBackend, executor};

#[tokio::test]
async fn blank_new_action_has_no_remote_or_journal_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(start_fake_server(&temp, &log).await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(FakeBackend::default());
    let executor = executor(&temp, db.clone(), bridge, backend).with_server(server.clone());
    let result = executor
        .execute_with_context(
            CommandAction::New {
                prompt: " \n\t".into(),
            },
            ActionContext {
                channel_id: 99,
                user_id: 20,
                discord_message_id: Some(30),
                auto_queue_when_busy: true,
            },
        )
        .await;
    assert!(
        result.is_err(),
        "blank prompt must be rejected before creating a thread"
    );
    assert!(!rpc_log(&log).iter().any(|r| r["method"] == "thread/start"));
    assert!(cdr_store::ingress::by_origin(&db, 30).unwrap().is_none());
    server.close().await.unwrap();
}

#[tokio::test]
async fn new_then_ask_reuses_the_directly_managed_thread_without_a_fork() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(start_fake_server(&temp, &log).await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(FakeBackend::default());
    let executor = executor(&temp, db.clone(), Arc::clone(&bridge), Arc::clone(&backend))
        .with_server(Arc::clone(&server));

    let created = executor
        .execute(
            CommandAction::New {
                prompt: "first".into(),
            },
            99,
            20,
        )
        .await
        .unwrap();
    assert_eq!(created.text, "In progress\nmessage: first");
    assert!(is_app_server_managed_target(&db, "new-thread").unwrap());
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("new-thread")
    );

    let asked = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "second".into(),
            },
            ActionContext {
                channel_id: 99,
                user_id: 20,
                discord_message_id: Some(30),
                auto_queue_when_busy: true,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        asked.text,
        "Queued\nmessage: 앞선 작업이 끝나면 시작합니다."
    );
    assert!(backend.forks.lock().await.is_empty());
    assert_eq!(
        *backend.starts.lock().await,
        vec![("new-thread".into(), "first".into())]
    );
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|job| job.target_thread_id == "new-thread"));
    assert!(
        rpc_log(&log)
            .iter()
            .any(|request| request["method"] == "thread/start")
    );
    server.close().await.unwrap();
}
