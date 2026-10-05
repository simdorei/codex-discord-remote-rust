//! S4-01 remainder: Running stop acceptance must not wait for the target owner.
use std::{sync::Arc, time::Duration};

use cdr_app_server::{ResidentAppServer, requests::AppRequest};
use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, list_filtered, mark_running};
use serde_json::json;

#[path = "support/action_app_server.rs"]
mod server_support;

struct Fixture {
    temp: tempfile::TempDir,
    db: std::path::PathBuf,
    log: std::path::PathBuf,
    server: Arc<ResidentAppServer>,
    queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    executor: ActionExecutor<AppServerTurnBackend>,
    original: serde_json::Value,
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let fence = Arc::new(
        RuntimeDeadGenerationFence::new(db.clone(), "p04-running-stop".into(), None).unwrap(),
    );
    let mut config = native_fixture::config("action");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        log.to_string_lossy().into_owned(),
    );
    let server = Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(config, fence)
            .await
            .unwrap(),
    );
    let backend = Arc::new(AppServerTurnBackend::new(Arc::clone(&server)));
    let queue = Arc::new(QueueCoordinator::new(db.clone(), backend));
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::clone(&queue),
    )
    .with_server(Arc::clone(&server));
    let generation = i64::try_from(server.generation()).unwrap();
    enqueue(
        &db,
        NewQueueJob {
            job_id: "running-a",
            target_thread_id: "thread-a",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(101),
            app_server_generation: generation,
            prompt: "original running input",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_attempt(&db, "running-a", &[], generation).unwrap();
    mark_running(&db, "running-a", "owned-a", generation).unwrap();
    server
        .execute(
            AppRequest {
                method: "test/active-turn",
                params: json!({"threadId":"thread-a","turnId":"owned-a"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    let original =
        serde_json::to_value(list_filtered(&db, Some("thread-a"), None).unwrap()).unwrap();

    Fixture {
        temp,
        db,
        log,
        server,
        queue,
        executor,
        original,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_stop_receipt_is_bounded_while_original_target_is_busy_and_b_progresses() {
    let Fixture {
        temp: _temp,
        db,
        log,
        server,
        queue,
        executor,
        original,
    } = fixture().await;
    // Delay the target owner, not the shared stdin transport.
    let guard = executor.control_lock("thread-a").await.unwrap();
    let delayed_owner = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(guard);
    });
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        executor.execute(
            CommandAction::Stop {
                reference: Some("thread-a".into()),
            },
            99,
            20,
        ),
    )
    .await;
    let b = tokio::time::timeout(
        Duration::from_secs(5),
        queue.submit("thread-b", 100, 21, None, "independent B"),
    )
    .await;
    delayed_owner.abort();
    let _ = delayed_owner.await;
    server.close().await.unwrap();
    let calls = server_support::rpc_log(&log);

    assert!(
        b.unwrap().unwrap().turn_id.is_some(),
        "independent B must progress"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|r| r["method"] == "turn/start" && r["params"]["threadId"] == "thread-b")
            .count(),
        1
    );
    assert_eq!(
        serde_json::to_value(list_filtered(&db, Some("thread-a"), None).unwrap()).unwrap(),
        original
    );
    assert!(
        !calls.iter().any(
            |r| matches!(r["method"].as_str(), Some("turn/start" | "thread/resume"))
                && r["params"]["threadId"] == "thread-a"
        ),
        "stop must not start/resume original A"
    );
    let receipt = result
        .expect("Running stop admission waited beyond three seconds behind the target owner")
        .expect("Running stop admission should persist intent without claiming execution ended");
    assert!(receipt.text.contains("Stop accepted"), "{}", receipt.text);
    assert!(!receipt.waits_for_final);
    assert!(
        cdr_store::execution_hold::reason(&db, "running-a")
            .unwrap()
            .is_some()
    );
}
