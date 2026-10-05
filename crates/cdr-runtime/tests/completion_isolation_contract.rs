use std::{sync::Arc, time::Duration};

use cdr_app_server::{Notification, ResidentAppServer, ResidentNotificationEvent};
use cdr_runtime::{
    app_backend::AppServerTurnBackend, completion_worker::run_completion_worker,
    queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_store::{delivery, delivery_receipt, mirror, queue};
use serde_json::json;
use tokio::sync::{broadcast, watch};

#[path = "support/completion_isolation_http.rs"]
mod http;

fn seed(
    coordinator: &QueueCoordinator<AppServerTurnBackend>,
    target: &str,
    channel: i64,
    job: &str,
    turn: &str,
    generation: u64,
) {
    let generation = i64::try_from(generation).unwrap();
    queue::enqueue(
        coordinator.db_path(),
        queue::NewQueueJob {
            job_id: job,
            target_thread_id: target,
            channel_id: channel,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "fixture input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(coordinator.db_path(), job, &[], generation).unwrap();
    queue::mark_running(coordinator.db_path(), job, turn, generation).unwrap();
}

fn terminal(target: &str, turn: &str, generation: u64) -> ResidentNotificationEvent {
    ResidentNotificationEvent::Notification {
        generation,
        notification: Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":target,"turn":{
                "id":turn,"status":"failed","error":{"message":"fixture terminal"}
            }}),
        },
    }
}

async fn native_server(temp: &tempfile::TempDir) -> Arc<ResidentAppServer> {
    let mut config = native_fixture::config("action");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    Arc::new(ResidentAppServer::start(config).await.unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_discord_a_does_not_delay_b_completion_storage_or_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = http::start().await;
    let server = native_server(&temp).await;
    let queue = Arc::new(QueueCoordinator::new(
        temp.path().join("mirror.sqlite"),
        Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
    ));
    let client = Arc::new(
        twilight_http::Client::builder()
            .token("test-token".into())
            .proxy(http.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let (events, receiver) = broadcast::channel(512);
    let (shutdown, stopped) = watch::channel(false);
    let running = tokio::spawn(run_completion_worker(
        receiver,
        Arc::clone(&server),
        Arc::clone(&queue),
        client,
        false,
        Duration::from_secs(2),
        stopped,
    ));
    seed(
        &queue,
        "isolate-a",
        42,
        "job-a",
        "turn-a",
        server.generation(),
    );
    events
        .send(terminal("isolate-a", "turn-a", server.generation()))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), &mut http.entered)
        .await
        .unwrap()
        .unwrap();
    // Only after the actual A POST is held for a programmed 30 seconds do we admit B.
    seed(
        &queue,
        "isolate-b",
        43,
        "job-b",
        "turn-b",
        server.generation(),
    );
    events
        .send(terminal("isolate-b", "turn-b", server.generation()))
        .unwrap();
    let b_ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let stored = mirror::has_event(
                queue.db_path(),
                &mirror::turn_origin_marker("isolate-b", "turn-b"),
                "isolate-b",
            )
            .unwrap();
            let b_posted = http.posts.lock().unwrap().iter().any(|p| p.channel == 43);
            let outbox_done = !delivery::list_pending(queue.db_path())
                .unwrap()
                .iter()
                .any(|p| p.job_id == "job-b");
            if stored && b_posted && outbox_done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let b_jobs = queue::list_filtered(queue.db_path(), Some("isolate-b"), None).unwrap();
    let still_waiting_a = !http.a_replied.load(std::sync::atomic::Ordering::SeqCst);
    let posts = http.posts.lock().unwrap().clone();
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap();
    let unknown = delivery_receipt::unknown_count(queue.db_path()).unwrap();
    server.close().await.unwrap();
    http.close().await;
    assert!(
        b_ready.is_ok(),
        "B exceeded 5s behind A's 30s HTTP: jobs={b_jobs:?}, posts={posts:?}"
    );
    assert!(b_jobs.is_empty(), "B completion must be durably consumed");
    assert!(still_waiting_a, "B proof must precede A's HTTP response");
    assert_eq!(posts.iter().filter(|p| p.channel == 42).count(), 1);
    assert_eq!(posts.iter().filter(|p| p.channel == 43).count(), 1);
    assert!(posts.iter().all(|p| p.content.starts_with("Failed")));
    assert_eq!(
        unknown, 1,
        "cancelled A delivery remains unknown, not resent"
    );
}
