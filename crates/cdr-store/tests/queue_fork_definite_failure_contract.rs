use cdr_store::delivery::{list_pending, stage_queue_completion};
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    AppServerForkHandoffError, DEFINITE_FORK_ERROR_PREFIX, NewAppServerForkHandoff, NewQueueJob,
    begin_app_server_fork_handoff, list, mark_running,
    record_and_cancel_app_server_fork_handoff_after_definite_failure,
    record_app_server_fork_failure, record_preflight_failure,
    repair_legacy_definite_app_server_fork_failures, stage_app_server_fork_target,
    try_begin_attempt, unresolved_app_server_fork_handoff_for_source,
};

#[test]
fn definite_failure_is_recorded_and_fence_is_released_in_one_operation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("atomic-definite.sqlite");
    setup_pending(&path, "atomic", 80);
    record_preflight_failure(&path, "pending", 4, "original queue error").unwrap();
    let expected = begin(&path, "atomic").handoff;
    let raw_error = format!("{}TAIL", "definite backend rejection ".repeat(80));

    let recorded = record_and_cancel_app_server_fork_handoff_after_definite_failure(
        &path, &expected, &raw_error,
    )
    .unwrap();
    assert_eq!(recorded.handoff_id, "atomic");
    assert_eq!(recorded.source_thread_id, "source");
    assert_eq!(recorded.last_fork_error.chars().count(), 1_000);
    assert_eq!(recorded.affected_jobs, 1);
    assert!(
        unresolved_app_server_fork_handoff_for_source(&path, "source")
            .unwrap()
            .is_none()
    );

    let pending = list(&path).unwrap().pop().unwrap();
    assert!(pending.last_error.starts_with(DEFINITE_FORK_ERROR_PREFIX));
    assert!(pending.last_error.contains("definite backend rejection"));
    assert!(pending.last_error.contains("original queue error"));
    let notices = list_pending(&path).unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].delivery_id, "fork-definite:atomic:pending");
    assert_eq!(notices[0].job_id, "fork-definite:atomic:pending");
    assert!(
        notices[0]
            .content
            .contains("failed before a target was created")
    );
    assert!(notices[0].content.contains("can be retried safely"));

    assert!(matches!(
        record_and_cancel_app_server_fork_handoff_after_definite_failure(
            &path,
            &expected,
            "duplicate",
        ),
        Err(AppServerForkHandoffError::ConflictingIntent { .. })
    ));
    assert_eq!(list_pending(&path).unwrap(), notices);
    try_begin_attempt(&path, "pending", &[], 4)
        .unwrap()
        .expect("released fence permits retry");
    mark_running(&path, "pending", "turn", 4).unwrap();
    stage_queue_completion(&path, "pending", "done", 99.0)
        .expect("synthetic failure notice does not block completion delivery");
    assert_eq!(list_pending(&path).unwrap().len(), 2);
}

#[test]
fn startup_repairs_the_legacy_record_then_cancel_crash_gap_once() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy-gap.sqlite");
    setup_pending(&path, "legacy", 81);
    begin(&path, "legacy");
    record_app_server_fork_failure(&path, "legacy", "legacy definite error", false).unwrap();
    assert!(
        try_begin_attempt(&path, "pending", &[], 4)
            .unwrap()
            .is_none()
    );
    assert!(list_pending(&path).unwrap().is_empty());

    let repaired = repair_legacy_definite_app_server_fork_failures(&path).unwrap();
    assert_eq!(repaired.len(), 1);
    assert_eq!(repaired[0].handoff_id, "legacy");
    assert_eq!(repaired[0].last_fork_error, "legacy definite error");
    assert!(
        unresolved_app_server_fork_handoff_for_source(&path, "source")
            .unwrap()
            .is_none()
    );
    assert!(
        list(&path).unwrap()[0]
            .last_error
            .contains("legacy definite error")
    );
    let notices = list_pending(&path).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(
        repair_legacy_definite_app_server_fork_failures(&path)
            .unwrap()
            .is_empty()
    );
    assert_eq!(list_pending(&path).unwrap(), notices);
    assert!(
        try_begin_attempt(&path, "pending", &[], 4)
            .unwrap()
            .is_some()
    );
}

#[test]
fn atomic_definite_cancellation_refuses_observed_or_ambiguous_handoffs() {
    let observed_dir = tempfile::tempdir().unwrap();
    let observed_path = observed_dir.path().join("observed.sqlite");
    setup_pending(&observed_path, "observed", 82);
    let observed = begin(&observed_path, "observed").handoff;
    stage_app_server_fork_target(&observed_path, "observed", "fork").unwrap();
    assert!(matches!(
        record_and_cancel_app_server_fork_handoff_after_definite_failure(
            &observed_path,
            &observed,
            "must refuse",
        ),
        Err(AppServerForkHandoffError::ForkTargetAlreadyObserved { .. })
    ));

    let ambiguous_dir = tempfile::tempdir().unwrap();
    let ambiguous_path = ambiguous_dir.path().join("ambiguous.sqlite");
    setup_pending(&ambiguous_path, "ambiguous", 83);
    let ambiguous = begin(&ambiguous_path, "ambiguous").handoff;
    record_app_server_fork_failure(&ambiguous_path, "ambiguous", "uncertain", true).unwrap();
    assert!(matches!(
        record_and_cancel_app_server_fork_handoff_after_definite_failure(
            &ambiguous_path,
            &ambiguous,
            "must refuse",
        ),
        Err(AppServerForkHandoffError::AmbiguousForkCannotBeCancelled { .. })
    ));
}

#[test]
fn atomic_definite_cancellation_refuses_a_changed_handoff_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("changed.sqlite");
    setup_pending(&path, "changed", 84);
    let expected = begin(&path, "changed").handoff;
    record_app_server_fork_failure(&path, "changed", "committed by another writer", false).unwrap();

    assert!(matches!(
        record_and_cancel_app_server_fork_handoff_after_definite_failure(
            &path,
            &expected,
            "stale caller",
        ),
        Err(AppServerForkHandoffError::ConflictingIntent { .. })
    ));
    assert!(
        unresolved_app_server_fork_handoff_for_source(&path, "source")
            .unwrap()
            .is_some()
    );
}

fn setup_pending(path: &std::path::Path, handoff_id: &str, message_id: i64) {
    upsert_thread(path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    cdr_store::queue::enqueue(path, job(message_id)).unwrap();
    let _ = handoff_id;
}

fn begin(path: &std::path::Path, handoff_id: &str) -> cdr_store::queue::BegunAppServerForkHandoff {
    begin_app_server_fork_handoff(
        path,
        NewAppServerForkHandoff {
            handoff_id,
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 4,
            quarantine_reason: "ownership fork",
        },
    )
    .unwrap()
}

fn job(message_id: i64) -> NewQueueJob<'static> {
    NewQueueJob {
        job_id: "pending",
        target_thread_id: "source",
        channel_id: 101,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        app_server_generation: 4,
        prompt: "keep",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}
