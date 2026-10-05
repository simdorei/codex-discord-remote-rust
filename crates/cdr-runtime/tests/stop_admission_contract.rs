//! S4-01: stop admission must not wait behind an unconfirmed target owner.
use std::{path::PathBuf, sync::Arc, time::Duration};

use cdr_app_server::ResidentAppServer;
use cdr_runtime::{
    action_executor::ActionExecutor, bridge_state::BridgeState, command_plan::CommandAction,
    queue_runner::QueueCoordinator,
};
use cdr_store::queue::{QueueJobState, begin_attempt, enqueue, list_filtered};

#[path = "support/action_app_server.rs"]
mod server_support;
#[path = "support/action_target.rs"]
mod support;

struct Fixture {
    _temp: tempfile::TempDir,
    database: PathBuf,
    executor: ActionExecutor<support::FakeBackend>,
    server: Arc<ResidentAppServer>,
    backend: Arc<support::FakeBackend>,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("mirror.sqlite");
        let server = Arc::new(
            server_support::start_fake_server(&temp, &temp.path().join("rpc.jsonl")).await,
        );
        let backend = Arc::new(support::FakeBackend::default());
        let executor = support::executor(
            &temp,
            database.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            Arc::clone(&backend),
        )
        .with_server(Arc::clone(&server));
        enqueue(
            &database,
            support::job("job-a", "thread-a", 99, "original input", 1.0),
        )
        .unwrap();
        Self {
            _temp: temp,
            database,
            executor,
            server,
            backend,
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(list_filtered(&self.database, Some("thread-a"), None).unwrap())
            .unwrap()
    }
}

fn stop() -> CommandAction {
    CommandAction::Stop {
        reference: Some("thread-a".into()),
    }
}

#[tokio::test]
async fn p04_stop_accepts_pending_without_claiming_that_execution_ended() {
    let fixture = Fixture::new().await;
    let before = fixture.snapshot();
    let result = fixture.executor.execute(stop(), 99, 20).await;
    fixture.server.close().await.unwrap();
    let result = result.expect("a pending request needs a durable stop, not an active-turn guess");
    assert!(result.text.contains("Stop accepted"), "{}", result.text);
    assert!(!result.waits_for_final);
    assert!(
        !server_support::rpc_log(&fixture.database.with_file_name("rpc.jsonl"))
            .iter()
            .any(|event| matches!(
                event["method"].as_str(),
                Some("turn/start" | "turn/interrupt" | "thread/resume")
            ))
    );
    assert_eq!(
        fixture.snapshot(),
        before,
        "stop must preserve the original request evidence"
    );
    assert!(
        cdr_store::execution_hold::reason(&fixture.database, "job-a")
            .unwrap()
            .is_some(),
        "a cold queue must see durable non-replay authority"
    );
    let cold = QueueCoordinator::new(fixture.database.clone(), Arc::clone(&fixture.backend));
    let _ = cold.kick_target("thread-a").await;
    assert!(fixture.backend.starts.lock().await.is_empty());
    assert_eq!(fixture.snapshot(), before);
}

#[tokio::test]
async fn p04_stop_is_accepted_within_three_seconds_while_target_lock_is_held() {
    let fixture = Fixture::new().await;
    let held = fixture.executor.control_lock("thread-a").await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        fixture.executor.execute(stop(), 99, 20),
    )
    .await;
    drop(held);
    fixture.server.close().await.unwrap();
    let result = result
        .expect("stop admission waited behind the target owner")
        .expect("stop admission failed");
    assert!(result.text.contains("Stop accepted"), "{}", result.text);
    assert!(
        cdr_store::execution_hold::reason(&fixture.database, "job-a")
            .unwrap()
            .is_some()
    );
    assert!(fixture.backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn p04_stop_preserves_starting_attempt_across_cold_coordinator() {
    let fixture = Fixture::new().await;
    begin_attempt(&fixture.database, "job-a", &[], 7).unwrap();
    let before = fixture.snapshot();
    let result = fixture.executor.execute(stop(), 99, 20).await;
    fixture.server.close().await.unwrap();
    assert!(
        result.is_ok(),
        "stop must record intent for unconfirmed Starting: {result:?}"
    );
    let cold = QueueCoordinator::new(fixture.database.clone(), Arc::clone(&fixture.backend));
    let _ = cold.kick_target("thread-a").await;
    let after = list_filtered(&fixture.database, Some("thread-a"), None).unwrap();
    assert_eq!(after[0].state, QueueJobState::Starting);
    assert_eq!(
        fixture.snapshot(),
        before,
        "never rewind or reissue Starting"
    );
    assert!(fixture.backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn p04_stop_rejects_foreign_owner_or_channel_without_holding_requests() {
    let fixture = Fixture::new().await;
    let before = fixture.snapshot();
    for (channel, owner) in [(99, 21), (98, 20)] {
        assert!(
            fixture
                .executor
                .execute(stop(), channel, owner)
                .await
                .is_err()
        );
    }
    fixture.server.close().await.unwrap();
    assert_eq!(fixture.snapshot(), before);
    assert!(
        cdr_store::execution_hold::reason(&fixture.database, "job-a")
            .unwrap()
            .is_none()
    );
    assert!(fixture.backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn p04_stop_storage_failure_is_not_an_acceptance_or_request_retry() {
    let fixture = Fixture::new().await;
    let before = fixture.snapshot();
    rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_stop BEFORE INSERT ON cdr_execution_holds
         BEGIN SELECT RAISE(ABORT,'injected stop write failure'); END;",
        )
        .unwrap();
    let result = fixture.executor.execute(stop(), 99, 20).await;
    fixture.server.close().await.unwrap();
    assert!(result.is_err());
    assert_eq!(fixture.snapshot(), before);
    assert!(
        cdr_store::execution_hold::reason(&fixture.database, "job-a")
            .unwrap()
            .is_none()
    );
    assert!(fixture.backend.starts.lock().await.is_empty());
}
