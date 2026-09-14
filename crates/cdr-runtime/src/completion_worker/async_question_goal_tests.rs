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

#[tokio::test]
async fn async_question_goal_handoff_preserves_question_during_progress_http_barrier() {
    let temp = tempfile::tempdir().unwrap();
    let (remote, progress_seen, release_progress) =
        approval_http::start_with_progress_barrier().await;
    let http = Arc::new(
        twilight_http::Client::builder()
            .token("fixture-token".into())
            .proxy(remote.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let mut config = native_fixture::config("async-question");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    control(&server, "test/active-turn").await;
    control(&server, "test/goal-on").await;
    let f = MessageFixture::with_server(&temp, http.clone(), server.clone());
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
    let tap = tokio::spawn(async move {
        let mut pending = pending;
        while let Some(event) = pending.recv().await {
            to_processor.send(event.clone()).await.unwrap();
            seen.send(event).await.unwrap();
        }
    });
    let processing = tokio::spawn(async move { process(&worker, processing_rx).await });
    control(&server, "test/finish-turn").await;
    tokio::time::timeout(Duration::from_secs(5), progress_seen)
        .await
        .unwrap()
        .unwrap();
    control(&server, "test/goal-next-question").await;
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
    let before = queue::list(db).unwrap();
    assert_eq!(before[0].turn_id.as_deref(), Some("original"));
    assert!(before[0].goal_waiting);
    let held: i64 = cdr_store::schema::open_initialized(db).unwrap().query_row(
        "SELECT COUNT(*) FROM cdr_async_question_inbox WHERE thread_id='thread-b' AND turn_id='goal-next' AND candidate_job_id='origin' AND state='waiting'",[],|r|r.get(0)).unwrap();
    assert_eq!(
        held, 2,
        "D1: both exact occurrences must already be durable before handoff"
    );
    let ids =
        [0, 1].map(|i| aq::occurrence_id("thread-b", "goal-next", "question-call", i).unwrap());
    assert!(
        ids.iter().all(|id| aq::get(db, id).is_err()),
        "no bound UI before exact handoff"
    );
    release_progress.send(()).unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(3), async {
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
    .await;
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
