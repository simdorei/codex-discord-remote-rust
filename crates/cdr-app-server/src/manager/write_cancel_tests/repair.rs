use super::*;

#[tokio::test]
async fn rr01_repair_rechecks_admission_after_waiting_for_the_actual_writer() {
    use std::sync::atomic::Ordering;
    let pause = Arc::new(WriteTestPause::new());
    let client = test_client(Arc::clone(&pause));
    let lock = client.inner.stdin.lock().await;
    let server = Arc::new(server_from_client(client.clone()));
    let valid = Arc::new(AtomicBool::new(true));
    let task = tokio::spawn({
        let server = Arc::clone(&server);
        let valid = Arc::clone(&valid);
        async move {
            server.request_for_tool_repair_checked("mcpServer/tool/call",
                json!({"threadId":"original","server":"node_repl","tool":"js_reset","arguments":{}}),
                Duration::from_secs(5), 1, Arc::new(move || {
                    if valid.load(Ordering::Acquire) { Ok(()) } else {
                        Err(AppServerError::InvalidReply { message: "admission remapped".into() })
                    }
                })).await
        }
    });
    timeout(Duration::from_secs(2), pause.wait_until_before_lock())
        .await
        .unwrap();
    valid.store(false, Ordering::Release);
    drop(lock);
    let result = timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(AppServerError::InvalidReply { .. })));
    assert!(
        timeout(Duration::from_millis(40), pause.wait_until_entered())
            .await
            .is_err()
    );
    assert!(!server.lifecycle_snapshot().await.quarantined);
    assert!(client.inner.pending.lock().unwrap().is_empty());
    server.close().await.unwrap();
}

#[tokio::test]
async fn repair_partial_write_still_quarantines_the_transport() {
    let pause = Arc::new(WriteTestPause::new());
    let server = Arc::new(test_server(Arc::clone(&pause)));
    let task = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .request_for_tool_repair(
                    "mcpServer/tool/call",
                    json!({"threadId":"a","server":"node_repl","tool":"js_reset","arguments":{}}),
                    Duration::from_secs(5),
                    1,
                )
                .await
        }
    });
    timeout(Duration::from_secs(2), pause.wait_until_entered())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(server.lifecycle_snapshot().await.quarantined);
    server.close().await.unwrap();
}

#[tokio::test]
async fn fully_flushed_repair_timeout_does_not_quarantine_b() {
    let pause = Arc::new(WriteTestPause::new());
    pause.release();
    let server = test_server(Arc::clone(&pause));
    let result = server
        .request_for_tool_repair(
            "mcpServer/tool/call",
            json!({"threadId":"a","server":"node_repl","tool":"js_reset","arguments":{}}),
            Duration::from_millis(80),
            1,
        )
        .await;
    assert!(matches!(result, Err(AppServerError::Timeout { .. })));
    let health = server.lifecycle_snapshot().await;
    assert!(!health.quarantined && !health.restart_pending);
    pause.release();
    let other = server
        .request(
            "thread/read",
            json!({"threadId":"b"}),
            Duration::from_millis(80),
            Some(1),
        )
        .await;
    assert!(
        matches!(other, Err(AppServerError::Timeout { .. })),
        "B was admitted, not fenced"
    );
    server.close().await.unwrap();
}

#[tokio::test]
async fn repair_rejects_global_reset_and_untargeted_requests() {
    let server = test_server(Arc::new(WriteTestPause::new()));
    for (method, params) in [
        ("config/mcpServer/reload", json!({"threadId":"a"})),
        (
            "mcpServer/tool/call",
            json!({"server":"node_repl","tool":"js_reset"}),
        ),
        (
            "mcpServer/tool/call",
            json!({"threadId":"a","server":"other","tool":"js_reset"}),
        ),
    ] {
        assert!(matches!(
            server
                .request_for_tool_repair(method, params, Duration::from_millis(80), 1)
                .await,
            Err(AppServerError::InvalidReply { .. })
        ));
    }
    server.close().await.unwrap();
}
