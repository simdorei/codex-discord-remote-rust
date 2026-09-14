use super::*;
use cdr_app_server::requests::{AppRequest, resume_thread, start_turn};
use cdr_store::idle_release as store;
use serde_json::{Value, json};

use crate::test_support::approval_http as http_fixture;

async fn fixture(temp: &tempfile::TempDir) -> CompletionWorker {
    let mut worker = goal_handoff_tests::make_worker(temp).await;
    worker.server.close().await.unwrap();
    let mut config = crate::test_support::native_fixture::config("idle-release");
    config.environment.insert(
        "IDLE_TEST_DIR".into(),
        temp.path().to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    worker.queue = Arc::new(QueueCoordinator::new(
        temp.path().join("mirror.sqlite"),
        Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
    ));
    crate::idle_release::install(&server, worker.queue.db_path()).unwrap();
    worker.server = server;
    goal_handoff_tests::setup_running(&worker);
    let mut notifications = worker.server.subscribe_notifications();
    worker
        .server
        .execute(
            AppRequest {
                method: "test/terminal",
                params: json!({"threadId":"thread"}),
                timeout: Duration::from_secs(2),
            },
            Some(1),
        )
        .await
        .unwrap();
    if let ResidentNotificationEvent::Notification {
        generation,
        notification,
    } = notifications.recv().await.unwrap()
    {
        worker
            .server
            .confirm_idle_observation(generation, &notification);
    } else {
        panic!("terminal witness missing");
    }
    worker
        .queue
        .stage_turn_completion("thread", "T1", "Final A")
        .await
        .unwrap();
    worker
}

fn trace(temp: &tempfile::TempDir) -> Vec<Value> {
    std::fs::read_to_string(temp.path().join("rpc.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn calls(temp: &tempfile::TempDir, method: &str) -> usize {
    trace(temp).iter().filter(|r| r["method"] == method).count()
}
async fn wait_for_call(temp: &tempfile::TempDir, method: &str) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while calls(temp, method) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn current(worker: &CompletionWorker) -> cdr_app_server::idle_release::IdleReleaseToken {
    crate::idle_release::token(
        store::get(worker.queue.db_path(), "thread")
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}

mod continuity;
mod eligibility;
mod isolation;
mod uncertainty;
