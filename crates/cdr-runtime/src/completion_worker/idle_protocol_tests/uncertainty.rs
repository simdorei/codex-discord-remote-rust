use super::*;

#[tokio::test]
async fn ir9_close_retries_exit_journal_after_actual_child_exit_without_rpc_replay() {
    let temp = tempfile::tempdir().unwrap();
    let worker = fixture(&temp).await;
    std::fs::write(temp.path().join("mode"), "bad_unsub").unwrap();
    worker.maintain_idle_subscriptions().await.unwrap();
    let original = current(&worker);
    assert_eq!(original.state, "Unknown");
    let before = trace(&temp);
    let db = rusqlite::Connection::open(worker.queue.db_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_exit_journal BEFORE UPDATE ON cdr_idle_release
        WHEN NEW.detail='OldServerExited' BEGIN SELECT RAISE(ABORT,'injected exit journal failure'); END;").unwrap();

    let error = worker.server.close().await.unwrap_err();
    assert!(error.to_string().contains("injected exit journal failure"));
    assert!(!worker.server.lifecycle_snapshot().await.healthy);
    assert_eq!(current(&worker), original);
    assert!(
        worker
            .server
            .execute(start_turn("thread", "blocked"), Some(1))
            .await
            .is_err()
    );

    db.execute_batch("DROP TRIGGER fail_exit_journal").unwrap();
    worker.server.close().await.unwrap();
    let settled = current(&worker);
    assert_eq!(settled.owner_id, original.owner_id);
    assert_eq!(settled.generation, original.generation);
    assert_eq!(settled.intent_id, original.intent_id);
    assert_eq!(
        settled.state, "Settled",
        "close lost retryable exact exit evidence"
    );
    assert_eq!(settled.detail, "OldServerExited");
    assert_eq!(
        trace(&temp),
        before,
        "close retry must not create or replay any RPC"
    );
}

#[tokio::test]
async fn ir5_ir9_unknown_survives_not_loaded_and_new_resident_while_old_owner_lives() {
    let temp = tempfile::tempdir().unwrap();
    let worker = fixture(&temp).await;
    std::fs::write(temp.path().join("mode"), "bad_unsub").unwrap();
    worker.maintain_idle_subscriptions().await.unwrap();
    assert_eq!(current(&worker).state, "Unknown");
    std::fs::write(temp.path().join("unloaded"), "unloaded").unwrap();
    worker
        .server
        .execute(
            cdr_app_server::requests::read_thread("thread", false),
            Some(1),
        )
        .await
        .unwrap();
    assert_eq!(current(&worker).state, "Unknown");
    let mut config = crate::test_support::native_fixture::config("idle-release");
    config.environment.insert(
        "IDLE_TEST_DIR".into(),
        temp.path().to_string_lossy().into_owned(),
    );
    let newer = ResidentAppServer::start(config).await.unwrap();
    crate::idle_release::install(&newer, worker.queue.db_path()).unwrap();
    assert_eq!(newer.generation(), worker.server.generation());
    assert_ne!(newer.instance_id(), worker.server.instance_id());
    assert!(
        newer
            .execute(start_turn("thread", "new prompt"), Some(1))
            .await
            .is_err()
    );
    assert!(
        newer
            .execute(resume_thread("thread"), Some(1))
            .await
            .is_err()
    );
    assert!(worker.server.lifecycle_snapshot().await.healthy);
    newer.close().await.unwrap();
    assert_eq!(current(&worker).state, "Unknown");
    assert_eq!(calls(&temp, "thread/resume"), 0);
    assert_eq!(calls(&temp, "turn/start"), 0);
    worker.server.close().await.unwrap();
    assert_eq!(current(&worker).detail, "OldServerExited");
}

#[tokio::test]
async fn ir12_resume_response_loss_and_cancel_hold_without_replay_or_start() {
    let temp = tempfile::tempdir().unwrap();
    let worker = fixture(&temp).await;
    worker.maintain_idle_subscriptions().await.unwrap();
    std::fs::write(temp.path().join("mode"), "drop_resume").unwrap();
    let server = Arc::clone(&worker.server);
    let caller =
        tokio::spawn(async move { server.execute(start_turn("thread", "input"), Some(1)).await });
    wait_for_call(&temp, "thread/resume").await;
    caller.abort();
    let _ = caller.await;
    assert_eq!(current(&worker).state, "Resubscribing");
    assert!(
        worker
            .server
            .execute(resume_thread("thread"), Some(1))
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while current(&worker).state == "Resubscribing" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(current(&worker).state, "Unknown");
    assert!(
        worker
            .server
            .execute(start_turn("thread", "retry"), Some(1))
            .await
            .is_err()
    );
    assert_eq!(calls(&temp, "thread/resume"), 1);
    assert_eq!(calls(&temp, "turn/start"), 0);
    worker.server.close().await.unwrap();
}

#[tokio::test]
async fn ir12_resume_success_commit_failure_keeps_hold_and_never_starts() {
    let temp = tempfile::tempdir().unwrap();
    let worker = fixture(&temp).await;
    worker.maintain_idle_subscriptions().await.unwrap();
    let db = rusqlite::Connection::open(worker.queue.db_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_resume_commit BEFORE UPDATE ON cdr_idle_release
        WHEN NEW.detail='SupersededByConfirmedResubscribe' BEGIN SELECT RAISE(ABORT,'injected commit failure'); END;").unwrap();
    assert!(
        worker
            .server
            .execute(start_turn("thread", "input"), Some(1))
            .await
            .unwrap_err()
            .to_string()
            .contains("injected commit failure")
    );
    assert_eq!(current(&worker).state, "Resubscribing");
    assert!(
        worker
            .server
            .execute(start_turn("thread", "retry"), Some(1))
            .await
            .is_err()
    );
    assert_eq!(calls(&temp, "thread/resume"), 1);
    assert_eq!(calls(&temp, "turn/start"), 0);
    worker.server.close().await.unwrap();
}
