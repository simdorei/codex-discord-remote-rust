use super::*;
use crate::test_support::approval_http as http;

#[tokio::test]
async fn held_start_and_its_failure_notice_survive_queue_generation_adoption() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"reject_next_start":"usage"}))
        .await;
    f.submit("held-before-restart").await;
    assert!(f.server.force_restart_if_quiescent().await.unwrap());
    f.controller.manual_override("thread-b").unwrap();
    f.configure(json!({"ordinary":true})).await;
    f.submit("new-after-restart").await;
    f.queue.recover().await.unwrap();
    assert_eq!(
        f.count("turn/start"),
        2,
        "original rejection must not be replayed"
    );
    let transport = http::start().await;
    let client = twilight_http::Client::builder()
        .proxy(transport.address.clone(), true)
        .ratelimiter(None)
        .build();
    let result = crate::completion_worker::deliver_start_failures(&f.db, &client).await;
    transport.stop.send(()).unwrap();
    let requests = transport.task.await.unwrap();
    assert!(
        result.is_ok(),
        "historical failed-start custody was lost after queue generation adoption: {result:?}"
    );
    assert_eq!(requests.len(), 1);
    assert!(
        cdr_store::reserve_policy::start_notice::pending(&f.db)
            .unwrap()
            .is_empty()
    );
    let held = cdr_store::queue::list(&f.db)
        .unwrap()
        .into_iter()
        .find(|j| j.job_id == "held-before-restart")
        .unwrap();
    assert!(held.turn_id.is_none());
    assert!(held.last_error.starts_with(reserve_policy::HOLD_PREFIX));
    f.close().await;
}

#[tokio::test]
async fn notice_insert_failure_is_not_replayed_after_cold_recovery_same_generation() {
    assert_notice_insert_failure_is_not_replayed(false).await;
}

#[tokio::test]
async fn notice_insert_failure_is_not_replayed_after_cold_recovery_new_generation() {
    assert_notice_insert_failure_is_not_replayed(true).await;
}

async fn failed_notice_fixture() -> (Fixture, cdr_store::queue::StoredQueueJob) {
    use cdr_store::{queue, reserve_policy::start_notice};

    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"reject_next_start":"usage"}))
        .await;
    let connection = rusqlite::Connection::open(&f.db).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_notice_insert BEFORE INSERT ON codex_reserve_start_notices \
             BEGIN SELECT RAISE(ABORT,'injected notice insert failure'); END;",
        )
        .unwrap();
    assert!(
        f.queue
            .submit_identified("notice-cold", "thread-b", 42, 3, None, "original request")
            .await
            .is_err()
    );
    let original = queue::list(&f.db).unwrap().remove(0);
    assert_eq!(original.state, queue::QueueJobState::Starting);
    assert_eq!(original.attempt_count, 1);
    assert!(original.turn_id.is_none());
    assert!(original.last_error.is_empty());
    assert!(
        cdr_store::new_reply::get(&f.db, &original.job_id)
            .unwrap()
            .is_none()
    );
    assert!(start_notice::pending(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    assert_eq!(
        reserve_policy::get(&f.db, "thread-b")
            .unwrap()
            .unwrap()
            .state,
        "ordinary"
    );
    connection
        .execute_batch("DROP TRIGGER reject_notice_insert;")
        .unwrap();
    (f, original)
}

async fn assert_notice_insert_failure_is_not_replayed(restart_resident: bool) {
    use cdr_store::{queue, reserve_policy::start_notice};

    let (f, original) = failed_notice_fixture().await;
    let connection = rusqlite::Connection::open(&f.db).unwrap();
    // Reconstruct all coordinator/controller state from the same durable DB.
    // The old fixture coordinator is never polled again. Only the offline child
    // is optionally restarted; no runtime or installed app-server is involved.
    if restart_resident {
        assert!(f.server.force_restart_if_quiescent().await.unwrap());
        assert_ne!(
            i64::try_from(f.server.generation()).unwrap(),
            original.app_server_generation
        );
    }
    f.configure(json!({"ordinary":true})).await;
    let controller = ReserveAutoController::new(f.server.clone(), f.db.clone());
    let recovered = QueueCoordinator::new_with_admission_gate(
        f.db.clone(),
        Arc::new(AppServerTurnBackend::new(f.server.clone()).with_reserve_auto(controller)),
        crate::restart_readiness::drain::AdmissionGate::new(),
    );
    let mut reports = Vec::new();
    for _ in 0..3 {
        // Expire both the original Starting lease and any incorrectly created
        // Pending backoff, without sleeping or altering the system clock.
        assert_eq!(
            connection
                .execute(
                    "UPDATE codex_turn_queue SET updated_at = 0 WHERE job_id = ?",
                    [&original.job_id],
                )
                .unwrap(),
            1
        );
        reports.push(recovered.recover().await.unwrap());
        recovered.kick_target("thread-b").await.unwrap();
    }
    reports.push(recovered.recover_target("thread-b").await.unwrap());
    let starts = f.count("turn/start");
    let settings_updates = f.count("thread/settings/update");
    let jobs = queue::list(&f.db).unwrap();
    let notices = start_notice::pending(&f.db).unwrap();
    let current_generation = i64::try_from(f.server.generation()).unwrap();
    let direct_retry =
        queue::try_begin_attempt(&f.db, &original.job_id, &[], current_generation).unwrap();
    f.close().await;

    // Check actual RPC count first: revision 3 reaches two, not merely a
    // different error string or intermediate state.
    assert_eq!(
        starts, 1,
        "the usage-rejected original request was replayed after cold recovery"
    );
    assert_eq!(settings_updates, 0);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, original.job_id);
    assert_eq!(jobs[0].state, queue::QueueJobState::Starting);
    assert_eq!(jobs[0].attempt_count, original.attempt_count);
    assert_eq!(
        jobs[0].app_server_generation,
        original.app_server_generation
    );
    assert_eq!(jobs[0].baseline_turn_ids, original.baseline_turn_ids);
    assert_eq!(jobs[0].prompt, original.prompt);
    assert!(jobs[0].turn_id.is_none());
    assert!(jobs[0].last_error.contains("does not authorize retry"));
    assert!(
        notices.is_empty(),
        "recovery must not invent a lost failure notice"
    );
    assert!(direct_retry.is_none());
    for report in reports {
        assert_eq!(report.requeued, 0);
        assert_eq!(report.started, 0);
        assert!(report.unresolved > 0);
    }
}
