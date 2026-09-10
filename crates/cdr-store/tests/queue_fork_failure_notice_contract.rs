use cdr_store::StoreError;
use cdr_store::delivery::{complete as complete_delivery, list_pending};
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    AppServerForkHandoffError, NewAppServerForkHandoff, NewQueueJob, QueueJobState,
    UNRESOLVED_FORK_ERROR_PREFIX, begin_app_server_fork_handoff, begin_attempt,
    cancel_app_server_fork_handoff_after_definite_failure, enqueue,
    finalize_app_server_fork_handoff, list, record_app_server_fork_failure,
    record_app_server_fork_finalize_failure, record_preflight_failure, record_start_failure,
    retract, stage_app_server_fork_target, unresolved_app_server_fork_handoff_for_source,
};

#[test]
fn ambiguous_fork_failure_is_durable_visible_idempotent_and_not_cancellable() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("failure.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("pending-a", 10)).unwrap();
    record_preflight_failure(&path, "pending-a", 4, "previous writer backoff").unwrap();
    begin_app_server_fork_handoff(&path, request()).unwrap();
    let raw_error = format!("{}TAIL", "transport failure ".repeat(80));

    let recorded = record_app_server_fork_failure(&path, "failure", &raw_error, true)
        .expect("ambiguous RPC failure stays on the intent");
    assert!(recorded.fork_failure_ambiguous);
    assert_eq!(recorded.last_fork_error.chars().count(), 1_000);
    let first = list(&path).unwrap();
    assert_eq!(first.len(), 1);
    assert!(
        first[0]
            .last_error
            .starts_with(UNRESOLVED_FORK_ERROR_PREFIX)
    );
    assert!(first[0].last_error.contains("transport failure"));
    assert!(first[0].last_error.contains("previous writer backoff"));
    let first_notice = list_pending(&path).unwrap();
    assert_eq!(first_notice.len(), 1);
    assert_eq!(first_notice[0].delivery_id, "fork-unresolved:pending-a");
    assert!(first_notice[0].content.contains("transport failure"));

    record_app_server_fork_failure(&path, "failure", &raw_error, true).unwrap();
    assert_eq!(list_pending(&path).unwrap(), first_notice);
    assert!(complete_delivery(&path, "fork-unresolved:pending-a").unwrap());
    record_app_server_fork_failure(&path, "failure", &raw_error, true).unwrap();
    assert!(list_pending(&path).unwrap().is_empty());

    assert!(matches!(
        enqueue(&path, job("pending-b", 11)),
        Err(StoreError::ForkHandoffUnresolved { .. })
    ));
    record_app_server_fork_failure(&path, "failure", "still uncertain", true).unwrap();
    assert!(list_pending(&path).unwrap().is_empty());
    assert!(matches!(
        cancel_app_server_fork_handoff_after_definite_failure(&path, "failure"),
        Err(AppServerForkHandoffError::AmbiguousForkCannotBeCancelled { .. })
    ));
    assert!(
        unresolved_app_server_fork_handoff_for_source(&path, "source")
            .unwrap()
            .is_some()
    );

    stage_app_server_fork_target(&path, "failure", "fork").unwrap();
    let completed = finalize_app_server_fork_handoff(&path, "failure", 9).unwrap();
    assert!(completed.handoff.fork_failure_ambiguous);
    assert!(
        completed
            .handoff
            .last_fork_error
            .contains("still uncertain")
    );
    assert!(list_pending(&path).unwrap().is_empty());
    let jobs = list(&path).unwrap();
    assert_eq!(jobs.len(), 1);
    assert!(jobs.iter().all(|job| job.target_thread_id == "fork"));
    assert!(jobs.iter().all(|job| job.last_error.is_empty()));
}

#[test]
fn restart_records_a_generic_interruption_and_notifies_the_ambiguous_starting_job_once() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("starting-notice.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("ambiguous", 20)).unwrap();
    begin_attempt(&path, "ambiguous", &[], 4).unwrap();
    record_start_failure(&path, "ambiguous", 4, "original resume timeout", true).unwrap();
    let begun = begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "starting",
            ambiguous_job_id: Some("ambiguous"),
            source_thread_id: "source",
            expected_generation: 4,
            quarantine_reason: "recover uncertain start",
        },
    )
    .unwrap();
    assert!(begun.handoff.last_fork_error.is_empty());

    let generic = "bridge restarted before the fork outcome was durably recorded";
    record_app_server_fork_failure(&path, "starting", generic, true).unwrap();
    let job = list(&path).unwrap().pop().unwrap();
    assert!(job.last_error.starts_with(UNRESOLVED_FORK_ERROR_PREFIX));
    assert!(job.last_error.contains(generic));
    assert!(job.last_error.contains("original resume timeout"));
    let notices = list_pending(&path).unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].delivery_id, "fork-unresolved-starting:ambiguous");
    assert_eq!(notices[0].job_id, "fork-unresolved-starting:ambiguous");
    record_app_server_fork_failure(&path, "starting", generic, true).unwrap();
    assert_eq!(list_pending(&path).unwrap(), notices);

    stage_app_server_fork_target(&path, "starting", "fork").unwrap();
    finalize_app_server_fork_handoff(&path, "starting", 9).unwrap();
    let deliveries = list_pending(&path).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].delivery_id, "quarantine:ambiguous");
    assert_eq!(deliveries[0].job_id, "ambiguous");
    assert_eq!(list(&path).unwrap()[0].state, QueueJobState::Quarantined);
}

#[test]
fn retract_rejects_an_unresolved_fork_and_preserves_its_pending_warning() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("retract-unresolved.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("pending", 30)).unwrap();
    begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "retract",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 4,
            quarantine_reason: "ownership fork",
        },
    )
    .unwrap();
    record_app_server_fork_failure(&path, "retract", "fork outcome interrupted", true).unwrap();
    let warning_before = list_pending(&path).unwrap();
    assert_eq!(warning_before.len(), 1);

    let result = retract(&path, "source", Some(101), Some(7));
    assert!(matches!(
        result,
        Err(StoreError::ForkHandoffUnresolved {
            target_thread_id,
            last_error: Some(error),
        }) if target_thread_id == "source" && error.contains("interrupted")
    ));
    assert_eq!(list(&path).unwrap().len(), 1);
    assert_eq!(list_pending(&path).unwrap(), warning_before);
}

#[test]
fn repeated_finalize_failure_replaces_undelivered_notice_without_recreating_delivered_notice() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("latest-finalize-error.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("pending-a", 40)).unwrap();
    record_preflight_failure(&path, "pending-a", 4, "original writer backoff").unwrap();
    begin_app_server_fork_handoff(&path, request()).unwrap();
    stage_app_server_fork_target(&path, "failure", "fork").unwrap();

    record_app_server_fork_finalize_failure(&path, "failure", "exact failure A").unwrap();
    let first_notice = list_pending(&path).unwrap();
    assert_eq!(first_notice.len(), 1);
    assert!(first_notice[0].content.contains("exact failure A"));

    record_app_server_fork_finalize_failure(&path, "failure", "exact failure B").unwrap();
    let job_after_b = list(&path).unwrap().pop().unwrap();
    assert!(job_after_b.last_error.contains("exact failure B"));
    assert!(job_after_b.last_error.contains("original writer backoff"));
    assert!(!job_after_b.last_error.contains("exact failure A"));
    assert_eq!(
        job_after_b
            .last_error
            .matches(UNRESOLVED_FORK_ERROR_PREFIX)
            .count(),
        1
    );
    let notice_after_b = list_pending(&path).unwrap();
    assert_eq!(notice_after_b.len(), 1);
    assert_eq!(notice_after_b[0].delivery_id, first_notice[0].delivery_id);
    assert!(notice_after_b[0].content.contains("exact failure B"));
    assert!(
        notice_after_b[0]
            .content
            .contains("original writer backoff")
    );
    assert!(!notice_after_b[0].content.contains("exact failure A"));

    assert!(complete_delivery(&path, "fork-unresolved:pending-a").unwrap());
    record_app_server_fork_finalize_failure(&path, "failure", "exact failure C").unwrap();
    assert!(list_pending(&path).unwrap().is_empty());
    let job_after_c = list(&path).unwrap().pop().unwrap();
    assert!(job_after_c.last_error.contains("exact failure C"));
    assert!(job_after_c.last_error.contains("original writer backoff"));
    assert!(!job_after_c.last_error.contains("exact failure B"));
    assert_eq!(
        job_after_c
            .last_error
            .matches(UNRESOLVED_FORK_ERROR_PREFIX)
            .count(),
        1
    );
}

fn request() -> NewAppServerForkHandoff<'static> {
    NewAppServerForkHandoff {
        handoff_id: "failure",
        ambiguous_job_id: None,
        source_thread_id: "source",
        expected_generation: 4,
        quarantine_reason: "ownership fork",
    }
}

fn job(id: &str, message_id: i64) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id: id,
        target_thread_id: "source",
        channel_id: 101,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        app_server_generation: 4,
        prompt: "keep",
        queued: true,
        ack_sent: true,
        created_at: f64::from(i32::try_from(message_id).unwrap()),
    }
}
