use super::*;
use crate::test_support::approval_http as http;
use cdr_store::reserve_policy::start_notice;

#[tokio::test]
async fn no_turn_failure_uses_real_http_receipt_once_even_when_dequeue_commit_fails() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"reject_next_start":"usage"}))
        .await;
    let failed = f.submit("notice-once").await;
    assert!(failed.turn_id.is_none());
    let transport = http::start().await;
    let client = twilight_http::Client::builder()
        .proxy(transport.address.clone(), true)
        .ratelimiter(None)
        .build();
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_notice_dequeue BEFORE DELETE ON codex_reserve_start_notices BEGIN SELECT RAISE(ABORT,'injected dequeue failure'); END;").unwrap();
    assert!(
        crate::completion_worker::deliver_start_failures(&f.db, &client)
            .await
            .is_err()
    );
    assert!(
        crate::completion_worker::deliver_start_failures(&f.db, &client)
            .await
            .is_err()
    );
    conn.execute_batch("DROP TRIGGER reject_notice_dequeue;")
        .unwrap();
    crate::completion_worker::deliver_start_failures(&f.db, &client)
        .await
        .unwrap();
    crate::completion_worker::deliver_start_failures(&f.db, &client)
        .await
        .unwrap();
    transport.stop.send(()).unwrap();
    let posts = transport.task.await.unwrap();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0].1["content"]
            .as_str()
            .unwrap()
            .starts_with("Failed\n")
    );
    assert!(start_notice::pending(&f.db).unwrap().is_empty());
    assert_eq!(cdr_store::queue::list(&f.db).unwrap().len(), 1);
    assert_eq!(f.count("turn/start"), 1);
    assert!(
        cdr_store::queue::try_begin_attempt(
            &f.db,
            "notice-once",
            &[],
            i64::try_from(f.server.generation()).unwrap()
        )
        .unwrap()
        .is_none()
    );
    f.close().await;
}

#[tokio::test]
async fn no_turn_notice_insert_failure_rolls_back_the_hold_instead_of_losing_the_notice() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"reject_next_start":"usage"}))
        .await;
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_notice_insert BEFORE INSERT ON codex_reserve_start_notices BEGIN SELECT RAISE(ABORT,'injected notice insert failure'); END;").unwrap();
    assert!(
        f.queue
            .submit_identified("notice-atomic", "thread-b", 42, 3, None, "test prompt")
            .await
            .is_err()
    );
    let jobs = cdr_store::queue::list(&f.db).unwrap();
    assert_eq!(jobs[0].state, cdr_store::queue::QueueJobState::Starting);
    assert!(jobs[0].turn_id.is_none());
    assert!(start_notice::pending(&f.db).unwrap().is_empty());
    conn.execute_batch("DROP TRIGGER reject_notice_insert;")
        .unwrap();
    f.queue.kick_target("thread-b").await.unwrap();
    assert_eq!(f.count("turn/start"), 1);
    f.close().await;
}

#[tokio::test]
async fn no_turn_receipt_is_blocked_when_its_original_room_is_remapped() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"reject_next_start":"usage"}))
        .await;
    f.submit("notice-remap").await;
    let transport = http::start().await;
    let client = twilight_http::Client::builder()
        .proxy(transport.address.clone(), true)
        .ratelimiter(None)
        .build();
    cdr_store::mapping::upsert_thread(&f.db, "thread-b", "project", "title", 100, 43, 2.0).unwrap();
    cdr_store::mapping::upsert_thread(&f.db, "foreign", "project", "other", 100, 42, 2.0).unwrap();
    assert!(
        crate::completion_worker::deliver_start_failures(&f.db, &client)
            .await
            .is_err()
    );
    assert_eq!(
        cdr_store::delivery_receipt::unknown_count(&f.db).unwrap(),
        0
    );
    transport.stop.send(()).unwrap();
    assert!(transport.task.await.unwrap().is_empty());
    assert_eq!(start_notice::pending(&f.db).unwrap().len(), 1);
    f.close().await;
}

#[tokio::test]
async fn transient_rate_limit_does_not_create_a_usage_hold_or_failure_notice() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"reject_next_start":"rate"}))
        .await;
    let failure = f.submit("transient").await;
    assert_eq!(
        failure.warning.unwrap().kind,
        crate::queue_runner::BackendFailureKind::Other
    );
    assert!(start_notice::pending(&f.db).unwrap().is_empty());
    assert!(
        !cdr_store::queue::list(&f.db).unwrap()[0]
            .last_error
            .starts_with(reserve_policy::HOLD_PREFIX)
    );
    assert_eq!(f.count("thread/settings/update"), 0);
    f.close().await;
}

#[tokio::test]
async fn quota_exhausted_or_missing_effort_never_mutates_settings() {
    for options in [
        json!({"used":100}),
        json!({"account":null}),
        json!({"missing_effort":true}),
    ] {
        let f = Fixture::new().await;
        f.configure(options).await;
        assert!(f.controller.prepare_turn("thread-b").await.is_err());
        assert_eq!(f.count("thread/settings/update"), 0);
        assert_eq!(f.count("turn/start"), 0);
        f.close().await;
    }
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":null})).await;
    assert!(f.controller.prepare_turn("thread-b").await.is_ok());
    assert_eq!(f.count("thread/settings/update"), 0);
    assert_eq!(f.count("turn/start"), 0);
    f.close().await;
}

#[tokio::test]
async fn no_op_resume_missing_reasoning_effort_cannot_borrow_a_stray_effort_field() {
    use crate::action_executor::settings_action::snapshot::Settings;
    let wrong = json!({"model":"gpt-reserve","effort":"high","serviceTier":"default","thread":{"id":"thread-b"}});
    assert!(Settings::from_resume(&wrong).is_err());
}
