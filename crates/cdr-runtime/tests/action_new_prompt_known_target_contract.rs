use std::sync::Arc;

use cdr_runtime::action_executor::ActionContext;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;

#[path = "support/action_target.rs"]
mod action_target;
#[path = "support/new_thread.rs"]
#[allow(dead_code)]
mod support;

#[tokio::test]
async fn intake_only_failure_keeps_known_created_thread_in_durable_hold() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::schema::open_initialized(&db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_new_intake BEFORE INSERT ON codex_prompt_intakes
         BEGIN SELECT RAISE(ABORT,'fixture intake rejected'); END;",
        )
        .unwrap();
    let log = temp.path().join("methods.log");
    let server = Arc::new(support::start_server(&log, "known-start-result").await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(action_target::FakeBackend::default());
    let executor = action_target::executor(&temp, db.clone(), bridge, backend.clone())
        .with_server(server.clone());
    let action = CommandAction::New {
        prompt: "retain raw first prompt".into(),
    };
    let context = ActionContext {
        channel_id: 99,
        user_id: 20,
        discord_message_id: Some(891),
        auto_queue_when_busy: false,
    };
    let error = executor
        .execute_with_context(action.clone(), context)
        .await
        .unwrap_err();
    let known = cdr_store::ingress::by_origin(&db, 891).unwrap().unwrap();
    let notices = cdr_store::delivery::list_pending(&db).unwrap();
    let repeated = executor.execute_with_context(action, context).await;
    server.close().await.unwrap();

    assert!(error.to_string().contains("fixture intake rejected"));
    assert!(repeated.is_err());
    assert_eq!(support::starts(&log), 1);
    assert!(backend.starts.lock().await.is_empty());
    assert!(
        cdr_store::prompt_intake::list_prompt_intakes(&db)
            .unwrap()
            .is_empty()
    );
    assert_eq!(known.state, "held");
    assert_eq!(known.payload["prompt"], "retain raw first prompt");
    assert_eq!(
        known.target_thread_id.as_deref(),
        Some("new-thread"),
        "IG-4: a known created thread must survive rollback of the separate prompt handoff"
    );
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.contains("new-thread"));
}
