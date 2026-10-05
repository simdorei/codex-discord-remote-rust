//! D1: actual observer can outrun the processor's goal ownership handoff.
use super::*;
use crate::{
    commentary_stream::CommentaryBuffer,
    completion_worker::terminal_fence::TerminalFence,
    soak::native_fixture,
    test_support::{approval_http, message_fixture::MessageFixture},
};
use cdr_app_server::{ResidentAppServer, requests::AppRequest};
use cdr_store::{async_question as aq, queue};
use serde_json::json;
use tokio::sync::Mutex;

async fn control(server: &ResidentAppServer, method: &'static str) {
    server
        .execute(
            AppRequest {
                method,
                params: json!({}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
}

async fn setup(
    temp: &tempfile::TempDir,
    http: Arc<twilight_http::Client>,
) -> (
    MessageFixture,
    Arc<ResidentAppServer>,
    Arc<CompletionWorker>,
) {
    let mut config = native_fixture::config("async-question");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    control(&server, "test/active-turn").await;
    control(&server, "test/goal-on").await;
    let f = MessageFixture::with_server(temp, http.clone(), server.clone());
    let db = f.executor.mirror_db();
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: "origin",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(40),
            app_server_generation: 1,
            prompt: "original goal",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, "origin", &[], 1).unwrap();
    queue::mark_running(db, "origin", "original", 1).unwrap();
    let worker = Arc::new(CompletionWorker {
        server: server.clone(),
        queue: f.queue.clone(),
        http,
        commentary_enabled: true,
        history_read_timeout: Duration::from_secs(2),
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: TerminalFence::default(),
    });
    (f, server, worker)
}

async fn wait_for_waiting_questions(db: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let held: i64 = cdr_store::schema::open_initialized(db).unwrap().query_row(
                "SELECT COUNT(*) FROM cdr_async_question_inbox WHERE thread_id='thread-b' AND turn_id='goal-next' AND candidate_job_id='origin' AND state='waiting'", [], |r| r.get(0)).unwrap();
            if held == 2 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}

async fn wait_for_observed_handoff(db: &std::path::Path, ids: &[String]) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if ids
                .iter()
                .all(|id| aq::get(db, id).is_ok_and(|q| q.state == "observed"))
                && queue::list(db).unwrap()[0].turn_id.as_deref() == Some("goal-next")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_open_questions(
    db: &std::path::Path,
    ids: &[String],
) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if ids
                .iter()
                .all(|id| aq::get(db, id).is_ok_and(|q| q.state == "open"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
}

async fn wait_for_completed_item(
    seen_rx: &mut mpsc::Receiver<cdr_app_server::ResidentNotificationEvent>,
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = seen_rx.recv().await {
            if let cdr_app_server::ResidentNotificationEvent::Notification { notification, .. } =
                event
                && notification.method == "item/completed"
            {
                break;
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn async_question_goal_handoff_preserves_question_during_progress_http_barrier() {
    let temp = tempfile::tempdir().unwrap();
    let (seen_progress, progress_seen) = tokio::sync::oneshot::channel();
    let (release_progress, resume_http) = tokio::sync::oneshot::channel();
    let remote =
        approval_http::start_with_progress_barrier(Some((seen_progress, resume_http))).await;
    let http = Arc::new(
        twilight_http::Client::builder()
            .token("fixture-token".into())
            .proxy(remote.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let (f, server, worker) = setup(&temp, http).await;
    let db = f.executor.mirror_db();
    let (stop, shutdown) = watch::channel(false);
    let (sender, pending) = mpsc::channel(128);
    let (seen, mut seen_rx) = mpsc::channel(128);
    let observed = tokio::spawn(observe(
        worker.clone(),
        server.subscribe_notifications(),
        sender,
        shutdown,
    ));
    // A tap proves the observer consumed/journalled Q while processing is blocked.
    let (to_processor, processing_rx) = mpsc::channel(128);
    let (release_start, start_ready) = tokio::sync::oneshot::channel();
    let tap = tokio::spawn(async move {
        let mut pending = pending;
        let mut start_ready = Some(start_ready);
        while let Some(event) = pending.recv().await {
            let observed = event.event.clone();
            if matches!(&observed, cdr_app_server::ResidentNotificationEvent::Notification { notification, .. }
                if notification.method == "turn/started")
                && let Some(ready) = start_ready.take()
            {
                ready.await.unwrap();
            }
            to_processor.send(event).await.unwrap();
            seen.send(observed).await.unwrap();
        }
    });
    let processing = tokio::spawn(async move { process(&worker, processing_rx).await });
    control(&server, "test/finish-turn").await;
    tokio::time::timeout(Duration::from_secs(5), progress_seen)
        .await
        .unwrap()
        .unwrap();
    // HTTP no longer gates local state. Hold the real target lease so the
    // observer-to-handoff boundary remains deterministic without weakening D1.
    let handoff_lock = f.queue.target_lock("thread-b").unwrap();
    let handoff_guard = handoff_lock.lock().await;
    control(&server, "test/goal-next-question").await;
    // Deterministically journal both questions before processing the queued start.
    wait_for_waiting_questions(db).await;
    assert_eq!(
        queue::list(db).unwrap()[0].turn_id.as_deref(),
        Some("original")
    );
    drop(handoff_guard);
    release_start.send(()).unwrap();
    wait_for_completed_item(&mut seen_rx).await;
    let ids =
        [0, 1].map(|i| aq::occurrence_id("thread-b", "goal-next", "question-call", i).unwrap());
    // State handoff is now independent of the held progress POST, while
    // same-channel question delivery must remain behind that exact receipt.
    wait_for_observed_handoff(db, &ids).await;
    let before = queue::list(db).unwrap();
    assert_eq!(before[0].turn_id.as_deref(), Some("goal-next"));
    assert!(!before[0].goal_waiting);
    assert_eq!(before[0].job_id, "origin");
    assert_eq!(
        cdr_store::delivery_receipt::unknown_count(db).unwrap(),
        1,
        "question POST must not overtake the one pending Goal progress receipt"
    );
    release_progress.send(()).unwrap();
    let delivered = wait_for_open_questions(db, &ids).await;
    stop.send(true).unwrap();
    observed.await.unwrap();
    tap.await.unwrap();
    processing.await.unwrap();
    server.close().await.unwrap();
    remote.stop.send(()).unwrap();
    let posts = remote.task.await.unwrap();
    assert_eq!(
        queue::list(db).unwrap()[0].turn_id.as_deref(),
        Some("goal-next")
    );
    assert!(
        delivered.is_ok(),
        "D1: confirmed goal successor questions were lost"
    );
    assert_eq!(
        posts
            .iter()
            .filter(|(_, b)| b["components"].as_array().is_some_and(|v| !v.is_empty()))
            .count(),
        2
    );
}
