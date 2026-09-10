use super::*;
use crate::{
    app_backend::AppServerTurnBackend, commentary_stream::CommentaryBuffer,
    queue_runner::QueueCoordinator,
};
use cdr_app_server::requests::AppRequest;
use serde_json::json;
use std::{
    future::{Future, poll_fn},
    sync::Arc,
    task::Poll,
    time::Duration,
};
use tokio::{net::TcpListener, sync::Mutex};
use twilight_http_ratelimiting::{Endpoint, Method, RateLimiter};

use crate::test_support::app_fixture;

async fn verify_revocation(close: bool) {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server =
        Arc::new(app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl")).await);
    let queue = Arc::new(QueueCoordinator::new(
        temp.path().join("mirror.sqlite"),
        Arc::new(AppServerTurnBackend::new(server.clone())),
    ));
    seed_active(&queue, &server).await;
    let limiter = RateLimiter::new(50);
    // A real Twilight permit holds the request before any network dispatch.
    let permit = limiter
        .acquire(Endpoint {
            method: Method::Post,
            path: "channels/42/typing".into(),
        })
        .await;
    let worker = CompletionWorker {
        server: server.clone(),
        queue,
        http: Arc::new(
            twilight_http::Client::builder()
                .token("test-token".into())
                .proxy(listener.local_addr().unwrap().to_string(), true)
                .ratelimiter(Some(limiter))
                .timeout(Duration::from_secs(1))
                .build(),
        ),
        commentary_enabled: false,
        history_read_timeout: Duration::from_secs(2),
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: crate::completion_worker::terminal_fence::TerminalFence::default(),
    };
    let mut typing = Box::pin(worker.send_typing());
    // Poll the actual worker until its HTTP select is pending. No timing sleep
    // or substitute sender: after the disk lookup, all preflight awaits are ready.
    tokio::time::timeout(
        Duration::from_secs(5),
        poll_fn(|cx| {
            assert!(
                typing.as_mut().poll(cx).is_pending(),
                "typing ended before the gate"
            );
            if worker.terminal_fence.pending_subscribers() > 0 {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
    if close {
        server.close().await.unwrap();
    } else {
        assert!(
            !server.force_restart_if_quiescent().await.unwrap(),
            "active turn prevents replacement, but restart revokes typing"
        );
    }
    let stopped = tokio::time::timeout(Duration::from_secs(1), &mut typing).await;
    // Always release the real permit, including the RED case.
    drop(permit);
    if !close {
        server.close().await.unwrap();
    }
    assert!(
        stopped.is_ok(),
        "resident revocation did not cancel pending typing"
    );
    stopped.unwrap().unwrap();
    assert!(
        !app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .any(|rpc| rpc["method"] == "turn/start")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "revoked request must not dispatch after the gate opens"
    );
}

#[tokio::test]
async fn closing_resident_cancels_typing_behind_real_rate_limit_gate() {
    verify_revocation(true).await;
}

#[tokio::test]
async fn pending_restart_cancels_typing_behind_real_rate_limit_gate() {
    verify_revocation(false).await;
}

async fn seed_active(
    queue: &QueueCoordinator<AppServerTurnBackend>,
    server: &cdr_app_server::ResidentAppServer,
) {
    cdr_store::queue::enqueue(
        queue.db_path(),
        cdr_store::queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "test",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(queue.db_path(), "job", &[], 1).unwrap();
    cdr_store::queue::mark_running(queue.db_path(), "job", "turn", 1).unwrap();
    server
        .execute(
            AppRequest {
                method: "test/active-turn",
                params: json!({"threadId":"thread","turnId":"turn"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
}
