use cdr_store::StoreError;
use cdr_store::delivery::{list_pending, stage_queue_completion};
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, get_prompt_intake, promote_prompt_intake_to_queue,
    try_claim_prompt_intake,
};
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, list, mark_running};

#[test]
fn immediate_real_completion_cannot_beat_atomic_intake_removal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("promotion.sqlite");
    let claim = claimed(&path, "job", "target", 101, 501, false);

    let promoted = promote_prompt_intake_to_queue(
        &path,
        &claim,
        queue_job("job", "target", 101, 501, "prepared:target:raw"),
        11.0,
    )
    .unwrap();

    assert!(promoted.created);
    assert_eq!(promoted.job.prompt, "prepared:target:raw");
    assert!(get_prompt_intake(&path, "job").unwrap().is_none());
    assert_eq!(list(&path).unwrap(), vec![promoted.job]);

    begin_attempt(&path, "job", &[], 7).unwrap();
    mark_running(&path, "job", "turn-job", 7).unwrap();
    stage_queue_completion(&path, "job", "one final answer", 12.0).unwrap();

    assert!(get_prompt_intake(&path, "job").unwrap().is_none());
    assert!(list(&path).unwrap().is_empty());
    let deliveries = list_pending(&path).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].job_id, "job");
}

#[test]
fn exact_existing_queue_occurrence_is_adopted_and_conflicting_prompt_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("existing.sqlite");
    let claim = claimed(&path, "job", "target", 101, 507, false);
    enqueue(&path, queue_job("job", "target", 101, 507, "prepared")).unwrap();

    let adopted = promote_prompt_intake_to_queue(
        &path,
        &claim,
        queue_job("job", "target", 101, 507, "prepared"),
        11.0,
    )
    .unwrap();
    assert!(!adopted.created);
    assert!(get_prompt_intake(&path, "job").unwrap().is_none());

    let conflict_path = temp.path().join("prompt-conflict.sqlite");
    let claim = claimed(&conflict_path, "job", "target", 101, 508, false);
    enqueue(
        &conflict_path,
        queue_job("job", "target", 101, 508, "old prepared"),
    )
    .unwrap();
    assert!(matches!(
        promote_prompt_intake_to_queue(
            &conflict_path,
            &claim,
            queue_job("job", "target", 101, 508, "new prepared"),
            11.0,
        ),
        Err(StoreError::PromptIntakeIdentityConflict { .. })
    ));
    assert!(get_prompt_intake(&conflict_path, "job").unwrap().is_some());
    assert_eq!(list(&conflict_path).unwrap()[0].prompt, "old prepared");
}

fn claimed(
    path: &std::path::Path,
    job_id: &str,
    target: &str,
    channel_id: i64,
    message_id: i64,
    require_current_mirror: bool,
) -> cdr_store::prompt_intake::PromptIntakeClaim {
    admit_prompt_intake(
        path,
        NewPromptIntake {
            job_id,
            target_thread_id: target,
            channel_id,
            owner_user_id: Some(7),
            discord_message_id: Some(message_id),
            raw_prompt: "raw prompt with attachment",
            auto_queue_when_busy: true,
            require_current_mirror,
            created_at: 1.0,
        },
    )
    .unwrap();
    try_claim_prompt_intake(path, job_id, 10.0, 20.0)
        .unwrap()
        .unwrap()
}

fn queue_job<'a>(
    job_id: &'a str,
    target: &'a str,
    channel_id: i64,
    message_id: i64,
    prompt: &'a str,
) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id,
        target_thread_id: target,
        channel_id,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        app_server_generation: 7,
        prompt,
        queued: false,
        ack_sent: true,
        created_at: 11.0,
    }
}
