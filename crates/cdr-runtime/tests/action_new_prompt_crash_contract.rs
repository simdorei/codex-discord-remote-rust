use std::sync::Arc;

use cdr_runtime::action_executor::ActionContext;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_store::prompt_intake::list_prompt_intakes;
use cdr_store::queue::list;
use tokio::time::timeout;

#[path = "support/action_target.rs"]
mod action_target;
#[path = "support/new_thread.rs"]
mod support;

use action_target::{FakeBackend, executor};
use support::{PreparationGate, TEST_TIMEOUT, start_server, starts, wait_for_start};

const RAW_PROMPT: &str = "  첫 요청\nkeep exact spacing and context\n";

#[tokio::test]
async fn journal_store_failure_prevents_outbound_thread_creation() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror-directory");
    std::fs::create_dir(&db).unwrap();
    let log = temp.path().join("methods.log");
    let server = Arc::new(start_server(&log, "known-start-result").await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(FakeBackend::default());
    let executor = executor(&temp, db, bridge, backend.clone()).with_server(server.clone());

    let result = executor.execute_with_context(new_action(), context()).await;
    server.close().await.unwrap();

    assert!(
        result.is_err(),
        "the actual SQLite path failure must surface"
    );
    assert_eq!(
        starts(&log),
        0,
        "thread/start cannot be sent before the raw request is durably journaled"
    );
    assert!(backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn cancelled_new_preparation_preserves_raw_prompt_for_same_target_startup_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("methods.log");
    let server = Arc::new(start_server(&log, "known-start-result").await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(FakeBackend::default());
    let gate = Arc::new(PreparationGate::new());
    let first_executor = Arc::new(
        executor(&temp, db.clone(), bridge.clone(), backend.clone())
            .with_server(server.clone())
            .with_prompt_preprocessor(gate.clone()),
    );
    let running = tokio::spawn({
        let first_executor = first_executor.clone();
        async move {
            first_executor
                .execute_with_context(new_action(), context())
                .await
        }
    });
    timeout(TEST_TIMEOUT, gate.entered.acquire())
        .await
        .expect("new-thread prompt preparation did not begin")
        .unwrap()
        .forget();
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    let persisted_before_recovery = list_prompt_intakes(&db).unwrap();
    let receipt_before_recovery = cdr_store::ingress::by_origin(&db, 881).unwrap().unwrap();
    let queue_before_recovery = list(&db).unwrap();
    drop(first_executor);

    let restarted = executor(&temp, db.clone(), bridge, backend.clone());
    let recovered = restarted.recover_prompt_intakes_on_startup().await;
    let jobs = list(&db).unwrap();
    let recorded_starts = backend.starts.lock().await.clone();
    server.close().await.unwrap();

    assert_eq!(
        persisted_before_recovery.len(),
        1,
        "the exact first prompt must already be durable before async preparation"
    );
    let intake = &persisted_before_recovery[0];
    assert_eq!(intake.raw_prompt, RAW_PROMPT);
    assert_eq!(intake.target_thread_id, "new-thread");
    assert_eq!(intake.channel_id, 99);
    assert_eq!(intake.owner_user_id, Some(20));
    assert_eq!(intake.discord_message_id, Some(881));
    assert_eq!(
        receipt_before_recovery.owner_id.as_deref(),
        Some(intake.job_id.as_str())
    );
    assert_eq!(
        receipt_before_recovery.target_thread_id.as_deref(),
        Some("new-thread")
    );
    assert!(queue_before_recovery.is_empty());
    assert_eq!(recovered.unwrap(), 1);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "new-thread");
    assert_eq!(jobs[0].prompt, RAW_PROMPT);
    assert_eq!(
        recorded_starts,
        vec![("new-thread".into(), RAW_PROMPT.into())]
    );
    assert!(backend.forks.lock().await.is_empty());
    assert_eq!(starts(&log), 1);
}

#[tokio::test]
async fn cancelled_unknown_thread_start_is_not_repeated_by_fresh_executor_redelivery() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("methods.log");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(FakeBackend::default());
    let server = Arc::new(start_server(&log, "unknown-start-result").await);
    let first_executor = Arc::new(
        executor(&temp, db.clone(), bridge.clone(), backend.clone()).with_server(server.clone()),
    );
    let running = tokio::spawn({
        let first_executor = first_executor.clone();
        async move {
            first_executor
                .execute_with_context(new_action(), context())
                .await
        }
    });
    wait_for_start(&log).await;
    let attempted = cdr_store::ingress::by_origin(&db, 881).unwrap().unwrap();
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    server.close().await.unwrap();
    drop(first_executor);
    drop(server);

    // A fresh resident removes transport quarantine as an accidental duplicate
    // guard. Only the durable original request may prevent a second creation.
    let replacement = Arc::new(start_server(&log, "known-start-result").await);
    let fresh_executor =
        executor(&temp, db.clone(), bridge, backend.clone()).with_server(replacement.clone());
    let redelivered = timeout(
        TEST_TIMEOUT,
        fresh_executor.execute_with_context(new_action(), context()),
    )
    .await
    .expect("redelivery did not resolve");
    let jobs = list(&db).unwrap();
    let held = cdr_store::ingress::by_origin(&db, 881).unwrap().unwrap();
    replacement.close().await.unwrap();

    assert_eq!(attempted.phase, "thread/start");
    assert_eq!(attempted.payload["prompt"], RAW_PROMPT);
    assert_eq!(attempted.payload["context"]["channel_id"], 99);
    assert_eq!(attempted.payload["context"]["user_id"], 20);
    assert_eq!(attempted.payload["context"]["discord_message_id"], 881);
    assert!(attempted.owner_id.is_none());
    assert_eq!(held.state, "held");
    assert_eq!(held.payload, attempted.payload);
    assert_eq!(
        starts(&log),
        1,
        "unknown thread/start must remain held instead of creating another thread on redelivery"
    );
    assert!(
        redelivered.is_err(),
        "unknown creation needs explicit manual review"
    );
    assert!(jobs.is_empty());
    assert!(backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn completed_new_request_does_not_recreate_after_queue_row_is_removed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("methods.log");
    let server = Arc::new(start_server(&log, "known-start-result").await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(FakeBackend::default());
    let executor = executor(&temp, db.clone(), bridge, backend.clone()).with_server(server.clone());
    executor
        .execute_with_context(new_action(), context())
        .await
        .unwrap();
    let job = list(&db).unwrap().remove(0);
    cdr_store::queue::complete(&db, &job.job_id).unwrap();

    let duplicate = executor.execute_with_context(new_action(), context()).await;
    server.close().await.unwrap();

    assert!(!duplicate.unwrap().waits_for_final);
    assert_eq!(starts(&log), 1);
    assert!(list(&db).unwrap().is_empty());
    assert_eq!(backend.starts.lock().await.len(), 1);
}

fn new_action() -> CommandAction {
    CommandAction::New {
        prompt: RAW_PROMPT.into(),
    }
}

#[tokio::test]
async fn direct_new_commands_without_event_ids_are_independent_requests() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("methods.log");
    let server = Arc::new(start_server(&log, "known-start-result").await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let executor = executor(&temp, db.clone(), bridge, Arc::new(FakeBackend::default()))
        .with_server(server.clone());
    for _ in 0..2 {
        executor.execute(new_action(), 99, 20).await.unwrap();
    }
    server.close().await.unwrap();

    let jobs = list(&db).unwrap();
    assert_eq!(starts(&log), 2);
    assert_eq!(jobs.len(), 2);
    assert_ne!(jobs[0].job_id, jobs[1].job_id);
    assert!(jobs.iter().all(|job| job.prompt == RAW_PROMPT));
}

fn context() -> ActionContext {
    ActionContext {
        channel_id: 99,
        user_id: 20,
        discord_message_id: Some(881),
        auto_queue_when_busy: false,
    }
}
