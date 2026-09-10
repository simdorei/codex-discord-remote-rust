use super::*;
use crate::test_support::approval_app_fixture as app;

#[tokio::test]
async fn restarted_prompt_worker_reuses_confirmed_message_receipt() {
    check_receipt(false).await;
}

#[tokio::test]
async fn failed_prompt_receipt_commit_never_authorizes_a_second_post() {
    check_receipt(true).await;
}

async fn check_receipt(fail_confirmation: bool) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log).await);
    let generation = server.generation();
    if fail_confirmation {
        cdr_store::schema::open_initialized(&db).unwrap().execute_batch(
            "CREATE TRIGGER reject_receipt BEFORE UPDATE OF message_id ON codex_delivery_receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END;"
        ).unwrap();
    }
    cdr_store::queue::enqueue(
        &db,
        cdr_store::queue::NewQueueJob {
            job_id: "owner",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(101),
            app_server_generation: i64::try_from(generation).unwrap(),
            prompt: "fixture",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(&db, "owner", &[], i64::try_from(generation).unwrap()).unwrap();
    cdr_store::queue::mark_running(&db, "owner", "turn-b", i64::try_from(generation).unwrap())
        .unwrap();
    server
        .request(
            "test/pending",
            serde_json::json!({}),
            Duration::from_secs(2),
            None,
        )
        .await
        .unwrap();
    let request = server
        .pending_server_requests(None)
        .await
        .unwrap()
        .remove(0);
    let gate = crate::test_support::http_gate::start().await;
    let http = Arc::new(
        Client::builder()
            .proxy(gate.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    gate.release.send(()).unwrap();
    for _ in 0..2 {
        let mut worker = ServerRequestWorker {
            server: server.clone(),
            mirror_db: db.clone(),
            http: http.clone(),
            sent: SentRequestCache::default(),
        };
        let result = worker.process(generation, request.clone()).await;
        if fail_confirmation {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
    }
    gate.entered.await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    server.close().await.unwrap();
    assert_eq!(
        posts.len(),
        1,
        "empty process cache does not authorize a fresh POST"
    );
    assert!(!posts[0]["components"].as_array().unwrap().is_empty());
    assert_eq!(
        cdr_store::delivery_receipt::unknown_count(&db).unwrap(),
        i64::from(fail_confirmation)
    );
}
