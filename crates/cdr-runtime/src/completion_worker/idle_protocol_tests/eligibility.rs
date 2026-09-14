use super::*;

#[tokio::test]
async fn ir7_readonly_eligibility_timeouts_keep_other_targets_usable() {
    for mode in ["drop_goal", "drop_read"] {
        let temp = tempfile::tempdir().unwrap();
        let worker = fixture(&temp).await;
        std::fs::write(temp.path().join("mode"), mode).unwrap();
        worker.maintain_idle_subscriptions().await.unwrap();
        assert_eq!(current(&worker).state, "Candidate");
        assert!(current(&worker).detail.contains("timed out"));
        assert!(worker.server.lifecycle_snapshot().await.healthy);
        assert_eq!(calls(&temp, "thread/unsubscribe"), 0);
        worker
            .server
            .execute(start_turn("B", "input"), Some(1))
            .await
            .unwrap();
        assert_eq!(calls(&temp, "turn/start"), 1);
        worker.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn ir3_goal_and_unverified_observation_prevent_any_unsubscribe() {
    for mode in ["bad_goal", "paused_goal", "wrong_turn", "gap"] {
        let temp = tempfile::tempdir().unwrap();
        let worker = fixture(&temp).await;
        std::fs::write(temp.path().join("mode"), mode).unwrap();
        if mode == "gap" {
            worker.server.mark_idle_observation_gap();
        }
        worker.maintain_idle_subscriptions().await.unwrap();
        assert_eq!(calls(&temp, "thread/unsubscribe"), 0, "{mode}");
        assert_eq!(current(&worker).state, "Candidate");
        assert!(!current(&worker).detail.is_empty());
        assert!(worker.server.lifecycle_snapshot().await.healthy);
        worker.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn ir3_pending_and_unscoped_requests_prevent_release() {
    for mode in ["pending_scoped", "pending_unscoped"] {
        let temp = tempfile::tempdir().unwrap();
        let worker = fixture(&temp).await;
        std::fs::write(temp.path().join("mode"), mode).unwrap();
        worker
            .server
            .execute(
                AppRequest {
                    method: "thread/read",
                    params: json!({"threadId":"thread","includeTurns":false}),
                    timeout: Duration::from_secs(2),
                },
                Some(1),
            )
            .await
            .unwrap();
        worker.maintain_idle_subscriptions().await.unwrap();
        assert_eq!(calls(&temp, "thread/unsubscribe"), 0);
        assert!(current(&worker).detail.contains("unsettled server request"));
        worker.server.close().await.unwrap();
    }
}
