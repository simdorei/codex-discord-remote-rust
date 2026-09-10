use super::*;
use crate::test_support::app_fixture;
use crate::{
    app_backend::AppServerTurnBackend, commentary_stream::CommentaryBuffer,
    queue_runner::QueueCoordinator,
};
use cdr_app_server::requests::AppRequest;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Mutex, oneshot},
};

#[tokio::test]
async fn one_channel_typing_rejection_does_not_starve_another_active_channel() {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, mut stopped) = oneshot::channel();
    let http_task = tokio::spawn(async move {
        let mut paths = Vec::new();
        loop {
            let (mut socket, _) =
                tokio::select! { _=&mut stopped=>break, v=listener.accept()=>v.unwrap() };
            let mut buffer = [0u8; 8192];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            let path = request.split_whitespace().nth(1).unwrap().to_owned();
            let rejected = path.contains("/42/");
            paths.push(path);
            let response = if rejected {
                "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: 24\r\nconnection: close\r\n\r\n{\"code\":0,\"message\":\"x\"}"
            } else {
                "HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            };
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        paths
    });
    let server =
        Arc::new(app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl")).await);
    let queue = Arc::new(QueueCoordinator::new(
        temp.path().join("mirror.sqlite"),
        Arc::new(AppServerTurnBackend::new(server.clone())),
    ));
    for (index, thread, channel) in [(1, "a", 42), (2, "b", 43)] {
        let job = format!("job-{index}");
        cdr_store::queue::enqueue(
            queue.db_path(),
            cdr_store::queue::NewQueueJob {
                job_id: &job,
                target_thread_id: thread,
                channel_id: channel,
                owner_user_id: Some(1),
                discord_message_id: None,
                app_server_generation: 1,
                prompt: "test",
                queued: false,
                ack_sent: true,
                created_at: f64::from(index),
            },
        )
        .unwrap();
        cdr_store::queue::begin_attempt(queue.db_path(), &job, &[], 1).unwrap();
        cdr_store::queue::mark_running(queue.db_path(), &job, "turn", 1).unwrap();
        server
            .execute(
                AppRequest {
                    method: "test/active-turn",
                    params: json!({"threadId":thread,"turnId":"turn"}),
                    timeout: Duration::from_secs(2),
                },
                None,
            )
            .await
            .unwrap();
    }
    let worker = CompletionWorker {
        server: server.clone(),
        queue,
        http: Arc::new(
            twilight_http::Client::builder()
                .token("test-token".into())
                .proxy(address, true)
                .ratelimiter(None)
                .timeout(Duration::from_secs(1))
                .build(),
        ),
        commentary_enabled: false,
        history_read_timeout: Duration::from_secs(2),
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: crate::completion_worker::terminal_fence::TerminalFence::default(),
    };
    assert!(
        worker.send_typing().await.is_err(),
        "the actual first error must remain visible"
    );
    stop.send(()).unwrap();
    let paths = http_task.await.unwrap();
    server.close().await.unwrap();
    assert!(
        !app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .any(|r| r["method"] == "turn/start")
    );
    assert_eq!(
        paths,
        vec!["/api/v10/channels/42/typing", "/api/v10/channels/43/typing"]
    );
}
