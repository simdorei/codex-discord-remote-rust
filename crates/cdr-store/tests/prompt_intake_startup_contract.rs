use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, get_prompt_intake, list_ready_prompt_intakes,
    record_prompt_intake_failure_if_claimed, release_all_prompt_intake_claims,
    remove_prompt_intake_if_queued, try_claim_prompt_intake,
};
use cdr_store::queue::{NewQueueJob, enqueue, list};

#[test]
fn singleton_startup_releases_claims_without_changing_backoff_or_queued_work() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("startup-release.sqlite");
    let fresh = admit_prompt_intake(&path, intake("fresh", 70))
        .unwrap()
        .intake;
    let fresh_claim = try_claim_prompt_intake(&path, "fresh", 20.0, 620.0)
        .unwrap()
        .unwrap();

    let backed = admit_prompt_intake(&path, intake("queued", 71))
        .unwrap()
        .intake;
    let first = try_claim_prompt_intake(&path, "queued", 1.0, 10.0)
        .unwrap()
        .unwrap();
    let failed =
        record_prompt_intake_failure_if_claimed(&path, &first, "preserve exact retry error", 20.0)
            .unwrap()
            .unwrap();
    let backed_claim = try_claim_prompt_intake(&path, "queued", 20.0, 620.0)
        .unwrap()
        .unwrap();
    enqueue(&path, queue_job(&backed)).unwrap();
    let queue_before = list(&path).unwrap();

    assert_eq!(release_all_prompt_intake_claims(&path).unwrap(), 2);
    assert_eq!(release_all_prompt_intake_claims(&path).unwrap(), 0);
    let fresh_after = get_prompt_intake(&path, "fresh").unwrap().unwrap();
    assert_eq!(fresh_after.claim_token, None);
    assert!((fresh_after.claim_expires_at - 0.0).abs() < f64::EPSILON);
    assert_eq!(fresh_after.last_error, fresh.last_error);
    assert_eq!(fresh_after.attempt_count, fresh.attempt_count);
    assert!((fresh_after.retry_after - fresh.retry_after).abs() < f64::EPSILON);
    assert!((fresh_after.updated_at - fresh_claim.intake.updated_at).abs() < f64::EPSILON);

    let backed_after = get_prompt_intake(&path, "queued").unwrap().unwrap();
    assert_eq!(backed_after.claim_token, None);
    assert!((backed_after.claim_expires_at - 0.0).abs() < f64::EPSILON);
    assert_eq!(backed_after.last_error, failed.last_error);
    assert_eq!(backed_after.attempt_count, failed.attempt_count);
    assert!((backed_after.retry_after - failed.retry_after).abs() < f64::EPSILON);
    assert!((backed_after.updated_at - backed_claim.intake.updated_at).abs() < f64::EPSILON);
    assert_eq!(list(&path).unwrap(), queue_before);

    let ready = list_ready_prompt_intakes(&path, 20.0).unwrap();
    assert_eq!(ready.len(), 2);
    assert!(
        try_claim_prompt_intake(&path, "fresh", 20.0, 620.0)
            .unwrap()
            .is_some()
    );
    assert!(remove_prompt_intake_if_queued(&path, "queued").unwrap());
    assert_eq!(list(&path).unwrap(), queue_before);
}

fn intake(job_id: &str, message_id: i64) -> NewPromptIntake<'_> {
    NewPromptIntake {
        job_id,
        target_thread_id: "source",
        channel_id: 101,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        raw_prompt: "attachment-enriched raw prompt",
        auto_queue_when_busy: true,
        require_current_mirror: true,
        created_at: 1.0,
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
