use cdr_store::delivery::{complete as complete_delivery, list_pending};
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, get_prompt_intake,
    record_prompt_intake_failure_if_claimed, try_claim_prompt_intake,
};
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, UNRESOLVED_FORK_ERROR_PREFIX,
    begin_app_server_fork_handoff, enqueue, finalize_app_server_fork_handoff, list,
    record_app_server_fork_cancellation_failure, record_app_server_fork_failure,
    record_app_server_fork_finalize_failure, stage_app_server_fork_target,
};

#[test]
fn prompt_only_ambiguous_failure_is_visible_latest_bounded_and_never_recreated() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("prompt-only.sqlite");
    seed_intake_with_error(&path, "intake", 10, "preprocessing backoff");
    begin_app_server_fork_handoff(&path, handoff("ambiguous")).unwrap();

    record_app_server_fork_failure(&path, "ambiguous", "fork timeout A", true).unwrap();
    assert!(
        list(&path).unwrap().is_empty(),
        "intakes are never startable queue rows"
    );
    let first = get_prompt_intake(&path, "intake").unwrap().unwrap();
    assert!(first.last_error.starts_with(UNRESOLVED_FORK_ERROR_PREFIX));
    assert!(first.last_error.contains("fork timeout A"));
    assert!(first.last_error.contains("preprocessing backoff"));
    assert_eq!(
        first
            .last_error
            .matches(UNRESOLVED_FORK_ERROR_PREFIX)
            .count(),
        1
    );
    assert!(first.last_error.chars().count() <= 1_000);
    let first_notice = list_pending(&path).unwrap();
    assert_eq!(first_notice.len(), 1);
    assert_eq!(first_notice[0].delivery_id, "fork-unresolved-intake:intake");
    assert_eq!(first_notice[0].job_id, "fork-unresolved-intake:intake");
    assert_ne!(first_notice[0].job_id, first.job_id);

    let claim = try_claim_prompt_intake(&path, "intake", 3.0, 10.0)
        .unwrap()
        .unwrap();
    let backed_off = record_prompt_intake_failure_if_claimed(
        &path,
        &claim,
        "runtime recovery observed the fence",
        20.0,
    )
    .unwrap()
    .unwrap();
    assert!(
        backed_off
            .last_error
            .starts_with(UNRESOLVED_FORK_ERROR_PREFIX)
    );
    assert!(backed_off.last_error.contains("fork timeout A"));
    assert_eq!(backed_off.attempt_count, 2);
    assert!((backed_off.retry_after - 20.0).abs() < f64::EPSILON);

    record_app_server_fork_failure(&path, "ambiguous", "fork timeout B", true).unwrap();
    let second = get_prompt_intake(&path, "intake").unwrap().unwrap();
    assert!(second.last_error.contains("fork timeout B"));
    assert!(!second.last_error.contains("fork timeout A"));
    assert!(second.last_error.contains("preprocessing backoff"));
    let second_notice = list_pending(&path).unwrap();
    assert_eq!(second_notice.len(), 1);
    assert_eq!(second_notice[0].delivery_id, first_notice[0].delivery_id);
    assert!(second_notice[0].content.contains("fork timeout B"));
    assert!(!second_notice[0].content.contains("fork timeout A"));

    assert!(complete_delivery(&path, "fork-unresolved-intake:intake").unwrap());
    record_app_server_fork_failure(&path, "ambiguous", "fork timeout C", true).unwrap();
    assert!(list_pending(&path).unwrap().is_empty());
    let third = get_prompt_intake(&path, "intake").unwrap().unwrap();
    assert!(third.last_error.contains("fork timeout C"));
    assert!(!third.last_error.contains("fork timeout B"));
    assert!(third.last_error.contains("preprocessing backoff"));
}

#[test]
fn prompt_only_finalize_and_cancellation_failures_stage_their_phase_notice() {
    let temp = tempfile::tempdir().unwrap();
    let finalize_path = temp.path().join("finalize.sqlite");
    admit_prompt_intake(&finalize_path, intake("finalize-job", 20)).unwrap();
    begin_app_server_fork_handoff(&finalize_path, handoff("finalize")).unwrap();
    stage_app_server_fork_target(&finalize_path, "finalize", "destination").unwrap();
    record_app_server_fork_finalize_failure(&finalize_path, "finalize", "mapping drift").unwrap();

    assert!(list(&finalize_path).unwrap().is_empty());
    let finalize_notice = list_pending(&finalize_path).unwrap();
    assert_eq!(finalize_notice.len(), 1);
    assert_eq!(
        finalize_notice[0].delivery_id,
        "fork-unresolved-intake:finalize-job"
    );
    assert!(finalize_notice[0].content.contains("finalization failed"));
    assert!(finalize_notice[0].content.contains("mapping drift"));

    let cancellation_path = temp.path().join("cancellation.sqlite");
    admit_prompt_intake(&cancellation_path, intake("cancel-job", 21)).unwrap();
    begin_app_server_fork_handoff(&cancellation_path, handoff("cancellation")).unwrap();
    record_app_server_fork_cancellation_failure(
        &cancellation_path,
        "cancellation",
        "fork rejected",
        "cancel commit failed",
    )
    .unwrap();

    assert!(list(&cancellation_path).unwrap().is_empty());
    let cancellation_notice = list_pending(&cancellation_path).unwrap();
    assert_eq!(cancellation_notice.len(), 1);
    assert_eq!(
        cancellation_notice[0].delivery_id,
        "fork-unresolved-intake:cancel-job"
    );
    assert!(
        cancellation_notice[0]
            .content
            .contains("cancellation could not be confirmed")
    );
    assert!(
        cancellation_notice[0]
            .content
            .contains("cancel commit failed")
    );
}

#[test]
fn queue_and_intake_on_the_same_source_each_keep_one_notice() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mixed.sqlite");
    admit_prompt_intake(&path, intake("intake", 30)).unwrap();
    enqueue(&path, queue_job("queued", 31)).unwrap();
    begin_app_server_fork_handoff(&path, handoff("mixed")).unwrap();

    record_app_server_fork_failure(&path, "mixed", "ambiguous transport", true).unwrap();

    let queue = list(&path).unwrap();
    assert_eq!(queue.len(), 1);
    assert!(
        queue[0]
            .last_error
            .starts_with(UNRESOLVED_FORK_ERROR_PREFIX)
    );
    let notices = list_pending(&path).unwrap();
    assert_eq!(notices.len(), 2);
    assert!(
        notices
            .iter()
            .any(|notice| notice.delivery_id == "fork-unresolved:queued")
    );
    assert!(notices.iter().any(|notice| {
        notice.delivery_id == "fork-unresolved-intake:intake"
            && notice.job_id == "fork-unresolved-intake:intake"
    }));
}

#[test]
fn successful_finalize_clears_an_undelivered_intake_notice_and_stale_marker() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("resolved.sqlite");
    admit_prompt_intake(&path, intake("intake", 40)).unwrap();
    begin_app_server_fork_handoff(&path, handoff("resolved")).unwrap();
    stage_app_server_fork_target(&path, "resolved", "destination").unwrap();
    record_app_server_fork_finalize_failure(&path, "resolved", "temporary mapping drift").unwrap();
    assert_eq!(list_pending(&path).unwrap().len(), 1);

    finalize_app_server_fork_handoff(&path, "resolved", 9).unwrap();

    assert!(list_pending(&path).unwrap().is_empty());
    let intake = get_prompt_intake(&path, "intake").unwrap().unwrap();
    assert_eq!(intake.target_thread_id, "destination");
    assert!(intake.last_error.is_empty());
}

fn seed_intake_with_error(path: &std::path::Path, job_id: &str, message_id: i64, error: &str) {
    admit_prompt_intake(path, intake(job_id, message_id)).unwrap();
    let claim = try_claim_prompt_intake(path, job_id, 1.0, 2.0)
        .unwrap()
        .unwrap();
    record_prompt_intake_failure_if_claimed(path, &claim, error, 0.0)
        .unwrap()
        .unwrap();
}

fn intake(job_id: &str, message_id: i64) -> NewPromptIntake<'_> {
    NewPromptIntake {
        job_id,
        target_thread_id: "source",
        channel_id: 101,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        raw_prompt: "durable enriched prompt",
        auto_queue_when_busy: true,
        require_current_mirror: false,
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

fn queue_job(job_id: &str, message_id: i64) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id,
        target_thread_id: "source",
        channel_id: 101,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        app_server_generation: 4,
        prompt: "prepared",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}
