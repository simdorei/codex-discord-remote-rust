use cdr_store::delivery::{list_pending, stage_queue_completion};
use cdr_store::mapping::upsert_thread;
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, canonicalize_prompt_intake_target, get_prompt_intake,
    list_prompt_intakes, list_ready_prompt_intakes, record_prompt_intake_failure_if_claimed,
    remove_prompt_intake_if_queued, try_claim_prompt_intake,
};
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, begin_app_server_fork_handoff, begin_attempt,
    cancel_app_server_fork_handoff_after_definite_failure, complete_app_server_fork_handoff,
    enqueue, finalize_app_server_fork_handoff, list, mark_running, record_app_server_fork_failure,
    stage_app_server_fork_target,
};
use cdr_store::schema::open_initialized;

#[test]
fn admission_is_idempotent_by_job_and_discord_message_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let first =
        admit_prompt_intake(&path, intake("job-a", Some(10), "original enriched raw")).unwrap();
    assert!(first.created);

    let by_job = admit_prompt_intake(&path, intake("job-a", Some(10), "changed body")).unwrap();
    assert!(!by_job.created);
    assert_eq!(by_job.intake, first.intake);
    let by_message =
        admit_prompt_intake(&path, intake("new-random-job", Some(10), "changed again")).unwrap();
    assert!(!by_message.created);
    assert_eq!(by_message.intake, first.intake);
    assert!(
        get_prompt_intake(&path, "new-random-job")
            .unwrap()
            .is_none()
    );
    assert_eq!(list_prompt_intakes(&path).unwrap(), vec![first.intake]);
}

#[test]
fn intake_survives_unmapped_begin_and_stage_then_finalize_retargets_it_once() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unmapped-fork.sqlite");
    let admitted = admit_prompt_intake(&path, intake("job", Some(20), "enriched\nattachment"))
        .unwrap()
        .intake;
    begin_app_server_fork_handoff(&path, handoff("handoff")).unwrap();
    assert_eq!(
        get_prompt_intake(&path, "job").unwrap(),
        Some(admitted.clone())
    );
    stage_app_server_fork_target(&path, "handoff", "destination").unwrap();
    assert_eq!(get_prompt_intake(&path, "job").unwrap(), Some(admitted));

    finalize_app_server_fork_handoff(&path, "handoff", 9).unwrap();
    let moved = get_prompt_intake(&path, "job").unwrap().unwrap();
    assert_eq!(moved.target_thread_id, "destination");
    assert_eq!(moved.raw_prompt, "enriched\nattachment");
    assert!(moved.require_current_mirror);
    assert!(moved.auto_queue_when_busy);

    let repeated = finalize_app_server_fork_handoff(&path, "handoff", 10).unwrap();
    assert!(!repeated.applied);
    assert_eq!(get_prompt_intake(&path, "job").unwrap(), Some(moved));
}

#[test]
fn admission_and_explicit_canonicalization_follow_only_completed_handoffs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("canonical.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    begin_app_server_fork_handoff(&path, handoff("completed")).unwrap();
    complete_app_server_fork_handoff(&path, "completed", "destination", 9).unwrap();

    let admitted = admit_prompt_intake(&path, intake("job", Some(30), "keep")).unwrap();
    assert_eq!(admitted.intake.target_thread_id, "destination");
    open_initialized(&path)
        .unwrap()
        .execute(
            "UPDATE codex_prompt_intakes SET target_thread_id = 'source' WHERE job_id = 'job'",
            [],
        )
        .unwrap();
    let canonical = canonicalize_prompt_intake_target(&path, "job")
        .unwrap()
        .unwrap();
    assert_eq!(canonical.target_thread_id, "destination");
}

#[test]
fn enqueue_then_cleanup_retry_is_idempotent_and_never_removes_intake_early() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cleanup.sqlite");
    let intake = admit_prompt_intake(&path, intake("fixed-job", Some(40), "raw"))
        .unwrap()
        .intake;
    assert!(!remove_prompt_intake_if_queued(&path, "fixed-job").unwrap());
    assert!(get_prompt_intake(&path, "fixed-job").unwrap().is_some());

    let first = enqueue(&path, queue_job(&intake)).unwrap();
    assert!(first.created);
    let retried_after_crash = enqueue(&path, queue_job(&intake)).unwrap();
    assert!(!retried_after_crash.created);
    assert_eq!(retried_after_crash.job.job_id, "fixed-job");
    assert_eq!(list(&path).unwrap().len(), 1);

    assert!(remove_prompt_intake_if_queued(&path, "fixed-job").unwrap());
    assert!(!remove_prompt_intake_if_queued(&path, "fixed-job").unwrap());
    assert!(list_prompt_intakes(&path).unwrap().is_empty());
    assert_eq!(list(&path).unwrap().len(), 1);
}

#[test]
fn legacy_cleanup_accepts_delivery_evidence_after_completion_removed_the_queue_row() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("delivery-cleanup.sqlite");
    let intake = admit_prompt_intake(&path, intake("fixed-job", Some(41), "raw"))
        .unwrap()
        .intake;
    enqueue(&path, queue_job(&intake)).unwrap();
    begin_attempt(&path, "fixed-job", &[], 9).unwrap();
    mark_running(&path, "fixed-job", "turn", 9).unwrap();
    stage_queue_completion(&path, "fixed-job", "done", 2.0).unwrap();

    assert!(list(&path).unwrap().is_empty());
    assert_eq!(list_pending(&path).unwrap().len(), 1);
    assert!(remove_prompt_intake_if_queued(&path, "fixed-job").unwrap());
    assert!(get_prompt_intake(&path, "fixed-job").unwrap().is_none());
}

#[test]
fn definite_fork_failure_stays_non_startable_and_durably_backed_off() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("fork-backoff.sqlite");
    admit_prompt_intake(&path, intake("job", Some(50), "must survive")).unwrap();
    let claim = try_claim_prompt_intake(&path, "job", 1.0, 90.0)
        .unwrap()
        .unwrap();
    assert!(list(&path).unwrap().is_empty());
    begin_app_server_fork_handoff(&path, handoff("failure")).unwrap();
    assert!(list(&path).unwrap().is_empty());
    record_app_server_fork_failure(&path, "failure", "fork was rejected", false).unwrap();
    assert!(list(&path).unwrap().is_empty());
    assert!(cancel_app_server_fork_handoff_after_definite_failure(&path, "failure").unwrap());

    let failed = record_prompt_intake_failure_if_claimed(&path, &claim, "fork was rejected", 100.0)
        .unwrap()
        .unwrap();
    assert_eq!(failed.attempt_count, 1);
    assert_eq!(failed.last_error, "fork was rejected");
    assert!((failed.retry_after - 100.0).abs() < f64::EPSILON);
    assert!(list_ready_prompt_intakes(&path, 99.999).unwrap().is_empty());
    assert_eq!(
        list_ready_prompt_intakes(&path, 100.0).unwrap(),
        vec![failed.clone()]
    );
    assert!(list(&path).unwrap().is_empty());

    let duplicate = admit_prompt_intake(&path, intake("job", Some(50), "replacement")).unwrap();
    assert!(!duplicate.created);
    assert_eq!(duplicate.intake, failed);
}

fn intake<'a>(job_id: &'a str, message_id: Option<i64>, raw: &'a str) -> NewPromptIntake<'a> {
    NewPromptIntake {
        job_id,
        target_thread_id: "source",
        channel_id: 101,
        owner_user_id: Some(7),
        discord_message_id: message_id,
        raw_prompt: raw,
        auto_queue_when_busy: true,
        require_current_mirror: true,
        created_at: 1.0,
    }
}

fn handoff(handoff_id: &str) -> NewAppServerForkHandoff<'_> {
    NewAppServerForkHandoff {
        handoff_id,
        ambiguous_job_id: None,
        source_thread_id: "source",
        expected_generation: 4,
        quarantine_reason: "ownership fork",
    }
}

fn queue_job(intake: &cdr_store::prompt_intake::StoredPromptIntake) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id: &intake.job_id,
        target_thread_id: &intake.target_thread_id,
        channel_id: intake.channel_id,
        owner_user_id: intake.owner_user_id,
        discord_message_id: intake.discord_message_id,
        app_server_generation: 9,
        prompt: "prepared for final target",
        queued: true,
        ack_sent: true,
        created_at: intake.created_at,
    }
}
