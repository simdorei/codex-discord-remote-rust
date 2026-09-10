use cdr_store::delivery::list_pending;
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, UNRESOLVED_FORK_ERROR_PREFIX,
    begin_app_server_fork_handoff, begin_attempt, enqueue, list,
    record_app_server_fork_cancellation_failure, record_app_server_fork_failure,
    record_preflight_failure, record_start_failure, stage_app_server_fork_target,
    unresolved_app_server_fork_handoff_for_source,
};

#[test]
fn unobserved_cancellation_failure_preserves_context_and_notifies_once() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unobserved.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("pending", 10)).unwrap();
    record_preflight_failure(&path, "pending", 4, "existing queue backoff").unwrap();
    begin_app_server_fork_handoff(&path, request("unobserved", None)).unwrap();
    record_app_server_fork_failure(&path, "unobserved", "earlier fork diagnostic", false).unwrap();

    let recorded = record_app_server_fork_cancellation_failure(
        &path,
        "unobserved",
        "definite thread/fork rejection",
        "SQLite cancellation commit failed",
    )
    .unwrap();
    assert!(recorded.fork_failure_ambiguous);
    assert_eq!(recorded.observed_target_thread_id, None);
    assert_eq!(recorded.target_thread_id, None);
    assert!(recorded.last_fork_error.contains("thread/fork rejection"));
    assert!(
        recorded
            .last_fork_error
            .contains("cancellation commit failed")
    );
    assert!(recorded.last_fork_error.contains("earlier fork diagnostic"));
    let queued = list(&path).unwrap();
    assert_eq!(queued.len(), 1);
    assert!(
        queued[0]
            .last_error
            .starts_with(UNRESOLVED_FORK_ERROR_PREFIX)
    );
    assert!(queued[0].last_error.contains("existing queue backoff"));
    let notices = list_pending(&path).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.contains("thread/fork rejection"));
    assert!(notices[0].content.contains("cancellation commit failed"));

    let repeated = record_app_server_fork_cancellation_failure(
        &path,
        "unobserved",
        "definite thread/fork rejection",
        "SQLite cancellation commit failed",
    )
    .unwrap();
    assert_eq!(repeated, recorded);
    assert_eq!(list_pending(&path).unwrap(), notices);
}

#[test]
fn observed_target_race_stays_recoverable_and_notifies_pending_and_starting_once() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("observed.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("ambiguous", 20)).unwrap();
    begin_attempt(&path, "ambiguous", &[], 4).unwrap();
    record_start_failure(&path, "ambiguous", 4, "resume timed out", true).unwrap();
    enqueue(&path, job("pending", 21)).unwrap();
    begin_app_server_fork_handoff(&path, request("observed", Some("ambiguous"))).unwrap();
    stage_app_server_fork_target(&path, "observed", "fork").unwrap();

    let recorded = record_app_server_fork_cancellation_failure(
        &path,
        "observed",
        "definite fork response",
        "cancel lost a race with target observation",
    )
    .expect("an observed but unfinalized target remains an unresolved fenced intent");
    assert!(recorded.fork_failure_ambiguous);
    assert_eq!(recorded.observed_target_thread_id.as_deref(), Some("fork"));
    assert_eq!(recorded.target_thread_id, None);
    assert!(
        unresolved_app_server_fork_handoff_for_source(&path, "source")
            .unwrap()
            .is_some()
    );
    let queued = list(&path).unwrap();
    assert_eq!(queued.len(), 2);
    assert!(
        queued
            .iter()
            .all(|job| job.last_error.starts_with(UNRESOLVED_FORK_ERROR_PREFIX))
    );
    let notices = list_pending(&path).unwrap();
    assert_eq!(notices.len(), 2);
    assert!(
        notices
            .iter()
            .any(|notice| notice.delivery_id == "fork-unresolved:pending")
    );
    assert!(
        notices
            .iter()
            .any(|notice| { notice.delivery_id == "fork-unresolved-starting:ambiguous" })
    );

    let repeated = record_app_server_fork_cancellation_failure(
        &path,
        "observed",
        "definite fork response",
        "cancel lost a race with target observation",
    )
    .unwrap();
    assert_eq!(repeated, recorded);
    assert_eq!(list_pending(&path).unwrap(), notices);
}

fn request<'a>(
    handoff_id: &'a str,
    ambiguous_job_id: Option<&'a str>,
) -> NewAppServerForkHandoff<'a> {
    NewAppServerForkHandoff {
        handoff_id,
        ambiguous_job_id,
        source_thread_id: "source",
        expected_generation: 4,
        quarantine_reason: "ownership recovery",
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
