use super::*;

#[tokio::test]
async fn ir8_ack_then_real_resume_once_before_one_start_old_closed_is_not_authority() {
    let temp = tempfile::tempdir().unwrap();
    let worker = fixture(&temp).await;
    worker.maintain_idle_subscriptions().await.unwrap();
    let old = current(&worker);
    assert_eq!(old.state, "AwaitUnload");
    worker
        .server
        .execute(start_turn("thread", "next"), Some(1))
        .await
        .unwrap();
    assert_eq!(current(&worker).detail, "SupersededByConfirmedResubscribe");
    assert_eq!(calls(&temp, "thread/resume"), 1);
    assert_eq!(calls(&temp, "turn/start"), 1);
    let methods: Vec<_> = trace(&temp)
        .into_iter()
        .filter_map(|v| v["method"].as_str().map(str::to_owned))
        .collect();
    assert!(
        methods.iter().position(|m| m == "thread/resume")
            < methods.iter().position(|m| m == "turn/start")
    );
    worker
        .server
        .execute(
            AppRequest {
                method: "test/closed",
                params: json!({"threadId":"thread"}),
                timeout: Duration::from_secs(2),
            },
            Some(1),
        )
        .await
        .unwrap();
    assert!(worker.server.release_idle_subscription(old).await.is_err());
    assert_eq!(current(&worker).detail, "SupersededByConfirmedResubscribe");
    worker.server.close().await.unwrap();
}

#[tokio::test]
async fn ir5_ack_alone_not_unload_but_fresh_read_after_ack_confirms_unload() {
    let temp = tempfile::tempdir().unwrap();
    let worker = fixture(&temp).await;
    worker.maintain_idle_subscriptions().await.unwrap();
    worker.maintain_idle_subscriptions().await.unwrap();
    assert_eq!(current(&worker).state, "AwaitUnload");
    std::fs::write(temp.path().join("unloaded"), "confirmed fixture state").unwrap();
    worker.maintain_idle_subscriptions().await.unwrap();
    assert_eq!(current(&worker).detail, "UnloadedConfirmed");
    assert_eq!(calls(&temp, "thread/unsubscribe"), 1);
    worker.server.close().await.unwrap();
}

#[tokio::test]
async fn ir6_http_failure_and_deleted_outbox_do_not_erase_release_or_replay_turn() {
    let temp = tempfile::tempdir().unwrap();
    let worker = fixture(&temp).await;
    let deliveries = cdr_store::delivery::list_pending(worker.queue.db_path()).unwrap();
    assert!(worker.deliver_one(&deliveries[0]).await.is_err());
    worker.maintain_idle_subscriptions().await.unwrap();
    let failed = cdr_store::delivery::list_pending(worker.queue.db_path()).unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].delivery_id, deliveries[0].delivery_id);
    assert!(!failed[0].last_error.is_empty());
    cdr_store::delivery::complete(worker.queue.db_path(), &failed[0].delivery_id).unwrap();
    assert_eq!(current(&worker).state, "AwaitUnload");
    assert_eq!(calls(&temp, "turn/start"), 0);
    worker.server.close().await.unwrap();
}
