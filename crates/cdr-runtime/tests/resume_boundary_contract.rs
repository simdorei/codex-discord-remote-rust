use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use std::{sync::Arc, time::Duration};
#[path = "support/resume_app_server.rs"]
mod support;

struct Fixture {
    _temp: tempfile::TempDir,
    log: std::path::PathBuf,
    server: Arc<cdr_app_server::ResidentAppServer>,
    bridge: Arc<BridgeState>,
    executor: Arc<ActionExecutor<AppServerTurnBackend>>,
}

impl Fixture {
    async fn new(scenario: &str, deadline: Duration) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(support::start(&temp, &log, scenario).await);
        let state = temp.path().join("state.sqlite");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        let db = temp.path().join("mirror.sqlite");
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        bridge.set_selected_thread_id(Some("thread-b")).unwrap();
        let queue = Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(AppServerTurnBackend::new(server.clone())),
        ));
        let executor = Arc::new(
            ActionExecutor::new(state, db, bridge.clone(), queue)
                .with_server(server.clone())
                .with_app_server_resume_timeout(deadline),
        );
        Self {
            _temp: temp,
            log,
            server,
            bridge,
            executor,
        }
    }

    fn start(
        &self,
    ) -> tokio::task::JoinHandle<
        Result<
            cdr_runtime::action_executor::ActionResult,
            cdr_runtime::action_executor::ActionError,
        >,
    > {
        let executor = self.executor.clone();
        tokio::spawn(async move {
            executor
                .execute(CommandAction::Resume { reference: None }, 99, 20)
                .await
        })
    }

    async fn wait_for_read(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !support::calls(&self.log)
                .iter()
                .any(|v| v["method"] == "thread/read")
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fixture must observe the real read RPC before revocation");
    }

    fn assert_read_only(&self, reads: usize) {
        let calls = support::calls(&self.log);
        assert_eq!(
            calls
                .iter()
                .filter(|v| v["method"] == "thread/read")
                .count(),
            reads
        );
        assert!(
            calls
                .iter()
                .all(|v| matches!(v["method"].as_str(), Some("initialize" | "thread/read")))
        );
    }
}

#[tokio::test]
async fn resume_control_lock_wait_has_a_deadline_without_rpc() {
    let fixture = Fixture::new("idle", Duration::from_millis(100)).await;
    let guard = fixture.executor.control_lock("thread-b").await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), fixture.start())
        .await
        .unwrap()
        .unwrap();
    drop(guard);
    fixture.server.close().await.unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("control wait timed out")
    );
    fixture.assert_read_only(0);
    assert_eq!(
        fixture.bridge.selected_thread_id().unwrap().as_deref(),
        Some("thread-b")
    );
}

#[tokio::test]
async fn resume_read_deadline_is_not_verified_success() {
    let fixture = Fixture::new("slow_read", Duration::from_millis(300)).await;
    let result = tokio::time::timeout(Duration::from_secs(2), fixture.start())
        .await
        .unwrap()
        .unwrap();
    fixture.server.close().await.unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("resume verification failed")
    );
    fixture.assert_read_only(1);
}

#[tokio::test]
async fn current_target_change_during_read_preserves_new_selection_and_reports_failure() {
    let fixture = Fixture::new("gated_read", Duration::from_secs(5)).await;
    let task = fixture.start();
    fixture.wait_for_read().await;
    fixture
        .bridge
        .set_selected_thread_id(Some("thread-a"))
        .unwrap();
    std::fs::write(fixture.log.with_extension("jsonl.release"), []).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    fixture.server.close().await.unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("target changed during verification")
    );
    assert_eq!(
        fixture.bridge.selected_thread_id().unwrap().as_deref(),
        Some("thread-a")
    );
    fixture.assert_read_only(1);
}

#[tokio::test]
async fn resident_close_during_read_never_claims_recovered_or_resends() {
    let fixture = Fixture::new("gated_read", Duration::from_secs(5)).await;
    let task = fixture.start();
    fixture.wait_for_read().await;
    fixture.server.close().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert_eq!(
        fixture.bridge.selected_thread_id().unwrap().as_deref(),
        Some("thread-b")
    );
    fixture.assert_read_only(1);
}
