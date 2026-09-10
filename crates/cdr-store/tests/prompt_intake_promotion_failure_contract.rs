use std::sync::{Arc, Barrier};

use cdr_store::StoreError;
use cdr_store::mapping::upsert_thread;
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, get_prompt_intake, promote_prompt_intake_to_queue,
    try_claim_prompt_intake,
};
use cdr_store::queue::{NewQueueJob, enqueue, list};
use cdr_store::schema::open_initialized;

#[test]
fn stale_claim_and_target_drift_roll_back_without_a_partial_queue_insert_or_delete() {
    let temp = tempfile::tempdir().unwrap();
    let stale_path = temp.path().join("stale.sqlite");
    let mut stale = claimed(&stale_path, "stale", "target", 101, 502, false);
    stale.claim_token = "not-current".into();

    assert!(matches!(
        promote_prompt_intake_to_queue(
            &stale_path,
            &stale,
            queue_job("stale", "target", 101, 502, "prepared"),
            11.0,
        ),
        Err(StoreError::PromptIntakeClaimLost { job_id }) if job_id == "stale"
    ));
    assert!(list(&stale_path).unwrap().is_empty());
    assert!(get_prompt_intake(&stale_path, "stale").unwrap().is_some());

    let moved_path = temp.path().join("moved.sqlite");
    let claim = claimed(&moved_path, "moved", "source", 101, 503, false);
    open_initialized(&moved_path)
        .unwrap()
        .execute(
            "UPDATE codex_prompt_intakes SET target_thread_id = 'destination' \
             WHERE job_id = 'moved'",
            [],
        )
        .unwrap();

    assert!(matches!(
        promote_prompt_intake_to_queue(
            &moved_path,
            &claim,
            queue_job("moved", "source", 101, 503, "prepared"),
            11.0,
        ),
        Err(StoreError::ForkHandoffTargetMoved {
            source_thread_id,
            target_thread_id,
        }) if source_thread_id == "source" && target_thread_id == "destination"
    ));
    assert!(list(&moved_path).unwrap().is_empty());
    assert_eq!(
        get_prompt_intake(&moved_path, "moved")
            .unwrap()
            .unwrap()
            .target_thread_id,
        "destination"
    );
}

#[test]
fn mirror_drift_and_queue_identity_conflict_keep_the_intake_and_queue_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let mirror_path = temp.path().join("mirror.sqlite");
    upsert_thread(
        &mirror_path,
        "different-target",
        "project",
        "Different",
        100,
        101,
        1.0,
    )
    .unwrap();
    let mirror = claimed(&mirror_path, "mirror", "source", 101, 504, true);

    assert!(matches!(
        promote_prompt_intake_to_queue(
            &mirror_path,
            &mirror,
            queue_job("mirror", "source", 101, 504, "prepared"),
            11.0,
        ),
        Err(StoreError::MirrorMappingChanged {
            discord_channel_id: 101,
            expected_target_thread_id,
            actual_target_thread_id: Some(actual),
        }) if expected_target_thread_id == "source" && actual == "different-target"
    ));
    assert!(list(&mirror_path).unwrap().is_empty());
    assert!(get_prompt_intake(&mirror_path, "mirror").unwrap().is_some());

    let retried = promote_prompt_intake_to_queue(
        &mirror_path,
        &mirror,
        queue_job("mirror", "different-target", 101, 504, "prepared:new"),
        11.0,
    )
    .expect("the claimed intake follows the transactionally current mirror");
    assert_eq!(retried.job.target_thread_id, "different-target");
    assert!(get_prompt_intake(&mirror_path, "mirror").unwrap().is_none());

    let conflict_path = temp.path().join("conflict.sqlite");
    let claim = claimed(&conflict_path, "same-job", "target", 101, 505, false);
    enqueue(
        &conflict_path,
        queue_job("same-job", "other-target", 999, 999, "other occurrence"),
    )
    .unwrap();
    assert!(matches!(
        promote_prompt_intake_to_queue(
            &conflict_path,
            &claim,
            queue_job("same-job", "target", 101, 505, "prepared"),
            11.0,
        ),
        Err(StoreError::PromptIntakeIdentityConflict { job_id, .. }) if job_id == "same-job"
    ));
    assert!(
        get_prompt_intake(&conflict_path, "same-job")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        list(&conflict_path).unwrap()[0].target_thread_id,
        "other-target"
    );
}

#[test]
fn concurrent_duplicate_promotion_has_one_commit_and_one_stale_claim_loser() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("concurrent.sqlite");
    let claim = Arc::new(claimed(&path, "job", "target", 101, 506, false));
    let barrier = Arc::new(Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let path = path.clone();
            let claim = Arc::clone(&claim);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                promote_prompt_intake_to_queue(
                    &path,
                    &claim,
                    queue_job("job", "target", 101, 506, "prepared"),
                    11.0,
                )
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::PromptIntakeClaimLost { .. })))
            .count(),
        1
    );
    assert!(get_prompt_intake(&path, "job").unwrap().is_none());
    assert_eq!(list(&path).unwrap().len(), 1);
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
