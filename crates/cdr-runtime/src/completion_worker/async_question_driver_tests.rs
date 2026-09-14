//! The production observer must journal before the processor renders real controls.
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
    server
        .execute(
            AppRequest {
                method: "test/active-turn",
                params: json!({}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
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
            prompt: "original prompt",
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

#[tokio::test]
async fn production_observer_and_processor_deliver_each_async_question_not_final() {
    let temp = tempfile::tempdir().unwrap();
    let remote = approval_http::start().await;
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
    let observed = tokio::spawn(observe(
        worker.clone(),
        server.subscribe_notifications(),
        sender,
        shutdown,
    ));
    let processing = tokio::spawn(async move { process(&worker, pending).await });
    server
        .execute(
            AppRequest {
                method: "test/question",
                params: json!({}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    let ids =
        [0, 1].map(|i| aq::occurrence_id("thread-b", "original", "question-call", i).unwrap());
    tokio::time::timeout(Duration::from_secs(5), async {
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
    .unwrap();
    assert_eq!(
        queue::list(db).unwrap()[0].turn_id.as_deref(),
        Some("original")
    );
    assert!(cdr_store::delivery::list_pending(db).unwrap().is_empty());
    stop.send(true).unwrap();
    observed.await.unwrap();
    processing.await.unwrap();
    server.close().await.unwrap();
    remote.stop.send(()).unwrap();
    let posts = remote.task.await.unwrap();
    assert_eq!(
        posts
            .iter()
            .filter(|(_, body)| body["content"]
                .as_str()
                .unwrap()
                .contains("어느 프로젝트인가요?"))
            .count(),
        1,
        "D2: the producer's original question context must appear exactly once"
    );
    assert_eq!(posts.len(), 5);
    assert!(
        posts
            .iter()
            .all(|(_, body)| !body["content"].as_str().unwrap().starts_with("Final"))
    );
    assert_eq!(
        posts[4].1["components"][0]["components"][1]["custom_id"],
        format!("codex_async:{}:1", ids[1])
    );
}
