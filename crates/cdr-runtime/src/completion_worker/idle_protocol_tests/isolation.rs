use super::*;

#[tokio::test]
async fn ir7_timeout_a_keeps_b_execution_approval_final_and_http_healthy() {
    let temp = tempfile::tempdir().unwrap();
    let mut worker = fixture(&temp).await;
    std::fs::write(temp.path().join("mode"), "drop_unsub").unwrap();
    let http = http_fixture::start().await;
    worker.http = Arc::new(
        Client::builder()
            .token("test-token".into())
            .proxy(http.address, true)
            .ratelimiter(None)
            .build(),
    );
    let server = Arc::clone(&worker.server);
    let token = current(&worker);
    let release = tokio::spawn(async move { server.release_idle_subscription(token).await });
    wait_for_call(&temp, "thread/unsubscribe").await;
    let result = release.await.unwrap();
    assert!(result.is_err());
    assert_eq!(current(&worker).state, "Unknown");
    assert!(worker.server.lifecycle_snapshot().await.healthy);
    worker
        .server
        .execute(start_turn("B", "input B"), Some(1))
        .await
        .unwrap();
    worker
        .server
        .execute(
            AppRequest {
                method: "test/pending",
                params: json!({"threadId":"B"}),
                timeout: Duration::from_secs(2),
            },
            Some(1),
        )
        .await
        .unwrap();
    let requests = worker
        .server
        .pending_server_requests(Some("B"))
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    worker
        .server
        .respond(
            &requests[0].id,
            requests[0].occurrence,
            json!({"answers":{}}),
            1,
        )
        .await
        .unwrap();
    let db = worker.queue.db_path();
    cdr_store::queue::enqueue(
        db,
        cdr_store::queue::NewQueueJob {
            job_id: "B-job",
            target_thread_id: "B",
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "B",
            queued: false,
            ack_sent: true,
            created_at: 3.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(db, "B-job", &[], 1).unwrap();
    cdr_store::queue::mark_running(db, "B-job", "T2", 1).unwrap();
    let delivery = worker
        .queue
        .stage_turn_completion_on_generation("B", "T2", "Final B", 1)
        .await
        .unwrap()
        .unwrap();
    worker.deliver_one(&delivery).await.unwrap();
    assert!(cdr_store::queue::list(db).unwrap().is_empty());
    assert_eq!(calls(&temp, "turn/start"), 1);
    assert!(
        worker
            .server
            .execute(start_turn("thread", "blocked"), Some(1))
            .await
            .is_err()
    );
    assert_eq!(calls(&temp, "turn/start"), 1);
    let _ = http.stop.send(());
    let traffic = http.task.await.unwrap();
    assert_eq!(traffic.len(), 1);
    assert!(
        traffic[0].1["content"]
            .as_str()
            .unwrap()
            .contains("Final B")
    );
    worker.server.close().await.unwrap();
    assert_eq!(current(&worker).detail, "OldServerExited");
}
