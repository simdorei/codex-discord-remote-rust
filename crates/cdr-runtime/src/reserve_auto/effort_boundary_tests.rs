//! Revision 15: real settings dispatch, durable intent and uncertainty boundaries.
use super::*;

async fn wait_gate(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !f.temp.path().join("gate-ready").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

fn release(f: &Fixture) {
    std::fs::write(f.temp.path().join("gate-release"), "release").unwrap();
}

#[tokio::test]
async fn r15_matching_default_reuse_never_sends_redundant_settings() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    let before = policy(&f);
    for _ in 0..3 {
        f.controller.prepare_turn("thread-b").await.unwrap();
    }
    let after = policy(&f);
    let updates = f.count("thread/settings/update");
    let starts = f.count("turn/start");
    f.close().await;
    assert_eq!(before, after);
    assert_eq!(updates, 1);
    assert_eq!(starts, 0);
}

#[tokio::test]
async fn r15_realign_lost_or_wrong_observation_is_not_retried_even_after_restart() {
    for options in [
        json!({"suppress_observation":true}),
        json!({"applied_effort_override":"xhigh"}),
    ] {
        let f = Fixture::new().await;
        f.controller.prepare_turn("thread-b").await.unwrap();
        reserve_policy::stage_usage_failure(&f.db, "thread-b", "prior failure").unwrap();
        f.configure(json!({"reserve_default":"high"})).await;
        f.configure(options).await;
        let result = submit_result(&f).await;
        let after = policy(&f);
        let first_repeat = f.controller.prepare_turn("thread-b").await;
        assert!(f.server.force_restart_if_quiescent().await.unwrap());
        let second_repeat = f.controller.prepare_turn("thread-b").await;
        let updates = f.count("thread/settings/update");
        let starts = f.count("turn/start");
        let pending = reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap();
        f.close().await;
        assert_held(&result);
        assert_eq!(after.state, "unknown");
        assert!(first_repeat.is_err());
        assert!(second_repeat.is_err());
        assert!(pending);
        assert_eq!(updates, 2);
        assert_eq!(starts, 0);
    }
}

#[tokio::test]
async fn r15_final_model_list_is_fresh_not_only_the_initial_catalog() {
    let f = Fixture::new().await;
    f.configure(json!({"reserve_default":"high","after_settings":{
        "reserve_efforts":supported(&["medium"])
    }}))
    .await;
    let result = submit_result(&f).await;
    let after = policy(&f);
    let updates = f.count("thread/settings/update");
    let starts = f.count("turn/start");
    let notice_count = notices(&f);
    f.close().await;
    assert_held(&result);
    assert_eq!(after.state, "unknown");
    assert_eq!(updates, 1);
    assert_eq!(starts, 0);
    assert_eq!(notice_count, 0);
}

#[tokio::test]
async fn r15_inflight_alignment_cancellation_retains_entering_intent_without_replay() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    let before = policy(&f);
    f.configure(json!({"reserve_default":"high","gate":"thread/settings/update"}))
        .await;
    let queue = f.queue.clone();
    let task = tokio::spawn(async move {
        queue
            .submit_identified("cancelled", "thread-b", 42, 3, None, "cancelled")
            .await
    });
    wait_gate(&f).await;
    let intent = policy(&f);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    release(&f);
    let repeat = f.controller.prepare_turn("thread-b").await;
    let updates = f.count("thread/settings/update");
    let starts = f.count("turn/start");
    let after = policy(&f);
    f.close().await;
    assert_eq!(intent.state, "entering");
    assert_eq!(intent.previous_model, before.previous_model);
    assert_eq!(intent.previous_effort, before.previous_effort);
    assert_eq!(after, intent);
    assert!(repeat.is_err());
    assert_eq!(updates, 2);
    assert_eq!(starts, 0);
}

#[tokio::test]
async fn r15_new_failure_during_alignment_cannot_be_resolved_by_the_old_claim() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    reserve_policy::stage_usage_failure(&f.db, "thread-b", "old failure").unwrap();
    f.configure(json!({"reserve_default":"high","gate":"thread/settings/update"}))
        .await;
    let queue = f.queue.clone();
    let task = tokio::spawn(async move {
        queue
            .submit_identified("inflight", "thread-b", 42, 3, None, "inflight")
            .await
    });
    wait_gate(&f).await;
    reserve_policy::stage_usage_failure(&f.db, "thread-b", "new failure").unwrap();
    let newer = reserve_policy::usage_failure_claim(&f.db, "thread-b")
        .unwrap()
        .unwrap();
    release(&f);
    let result = task.await.unwrap();
    let retained = reserve_policy::usage_failure_claim(&f.db, "thread-b")
        .unwrap()
        .unwrap();
    let after = policy(&f);
    let starts = f.count("turn/start");
    f.close().await;
    assert_held(&result);
    assert_eq!(after.state, "unknown");
    assert_eq!(retained.fence_id, newer.fence_id);
    assert_eq!(
        retained.failure_policy_revision,
        newer.failure_policy_revision
    );
    assert_eq!(starts, 0);
}

#[tokio::test]
async fn r15_superseding_off_policy_survives_old_alignment_completion() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"reserve_default":"high","gate":"thread/settings/update"}))
        .await;
    let queue = f.queue.clone();
    let task = tokio::spawn(async move {
        queue
            .submit_identified("inflight", "thread-b", 42, 3, None, "inflight")
            .await
    });
    wait_gate(&f).await;
    let off = reserve_policy::set_mode(&f.db, "thread-b", "off").unwrap();
    release(&f);
    let result = task.await.unwrap();
    let after = policy(&f);
    let starts = f.count("turn/start");
    f.close().await;
    assert_held(&result);
    assert_eq!(after, off);
    assert_eq!(after.mode, "off");
    assert_eq!(after.state, "entering");
    assert_eq!(starts, 0);
}

#[tokio::test]
async fn r15_alignment_begin_and_finish_store_abort_never_authorize_replay() {
    for finish_failure in [false, true] {
        let f = Fixture::new().await;
        f.controller.prepare_turn("thread-b").await.unwrap();
        reserve_policy::stage_usage_failure(&f.db, "thread-b", "prior failure").unwrap();
        let before = policy(&f);
        let condition = if finish_failure {
            "OLD.state='entering' AND NEW.state='reserve'"
        } else {
            "OLD.state='reserve' AND NEW.state='entering'"
        };
        let db = rusqlite::Connection::open(&f.db).unwrap();
        db.execute_batch(&format!(
            "CREATE TRIGGER fail_alignment BEFORE UPDATE OF state ON codex_reserve_policy
            WHEN {condition} BEGIN SELECT RAISE(ABORT,'fixture alignment failure'); END;"
        ))
        .unwrap();
        f.configure(json!({"reserve_default":"high"})).await;
        let result = submit_result(&f).await;
        let after = policy(&f);
        let pending = reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap();
        db.execute_batch("DROP TRIGGER fail_alignment;").unwrap();
        if finish_failure {
            assert!(f.controller.prepare_turn("thread-b").await.is_err());
        }
        let updates = f.count("thread/settings/update");
        let starts = f.count("turn/start");
        f.close().await;
        assert!(result.is_err());
        assert!(pending);
        if finish_failure {
            assert_eq!(after.state, "unknown");
        } else {
            assert_eq!(after, before);
        }
        assert_eq!(updates, if finish_failure { 2 } else { 1 });
        assert_eq!(starts, 0);
    }
}

#[tokio::test]
async fn r15_alignment_with_legacy_fence_keeps_the_request_held() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    reserve_policy::stage_usage_failure(&f.db, "thread-b", "legacy failure").unwrap();
    rusqlite::Connection::open(&f.db).unwrap().execute(
        "UPDATE codex_reserve_policy SET usage_failure_revision=NULL WHERE thread_id='thread-b'", [],
    ).unwrap();
    f.configure(json!({"reserve_default":"high"})).await;
    let result = submit_result(&f).await;
    let after = policy(&f);
    let pending = reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap();
    let starts = f.count("turn/start");
    f.close().await;
    assert_held(&result);
    assert_eq!(after.state, "unknown");
    assert!(pending);
    assert_eq!(starts, 0);
}
