use std::path::Path;

use cdr_store::dead_generation::{
    DeadGenerationCapture, activate_runtime, capture_dead_generation,
};
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, canonicalize_prompt_intake_target, get_prompt_intake,
    promote_prompt_intake_to_queue, try_claim_prompt_intake,
};
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, attach_goal_turn, begin_app_server_fork_handoff,
    complete, enqueue, list, mark_goal_waiting, mark_running_if_claimed,
    record_start_failure_if_claimed, try_begin_attempt,
};

fn job(job_id: &str) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id,
        target_thread_id: "held-thread",
        channel_id: 10,
        owner_user_id: Some(20),
        discord_message_id: None,
        app_server_generation: 1,
        prompt: "do not replay",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}

fn hold(db: &Path) {
    capture_dead_generation(
        db,
        DeadGenerationCapture {
            runtime_id: "runtime-a",
            generation: 1,
            snapshot_json: "{}",
            affected_targets: &["held-thread".into()],
            startup_channel_id: Some(88),
            has_unscoped_requests: false,
            now: 3.0,
        },
    )
    .unwrap();
}

#[test]
fn late_completion_and_attempt_results_cannot_mutate_held_original_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    activate_runtime(&db, "runtime-a").unwrap();
    enqueue(&db, job("running")).unwrap();
    let claim = try_begin_attempt(&db, "running", &[], 1).unwrap().unwrap();
    mark_running_if_claimed(&db, &claim, "turn-1")
        .unwrap()
        .unwrap();
    enqueue(&db, job("starting")).unwrap();
    let claim = try_begin_attempt(&db, "starting", &[], 1).unwrap().unwrap();
    hold(&db);
    let before = list(&db).unwrap();
    assert!(!complete(&db, "running").unwrap());
    assert!(!mark_goal_waiting(&db, "running", "turn-1", 1).unwrap());
    assert!(!attach_goal_turn(&db, "held-thread", "turn-2", 1).unwrap());
    assert!(
        cdr_store::delivery::stage_queue_completion(&db, "running", "late result", 4.0).is_err()
    );
    assert!(
        mark_running_if_claimed(&db, &claim, "late-turn")
            .unwrap()
            .is_none()
    );
    assert!(
        record_start_failure_if_claimed(&db, &claim, "late failure", false)
            .unwrap()
            .is_none()
    );
    assert_eq!(list(&db).unwrap(), before);
    assert!(
        begin_app_server_fork_handoff(
            &db,
            NewAppServerForkHandoff {
                handoff_id: "forbidden-fork",
                ambiguous_job_id: Some("starting"),
                source_thread_id: "held-thread",
                expected_generation: 1,
                quarantine_reason: "must not be treated as a generic quarantine",
            }
        )
        .is_err()
    );
}

#[test]
fn already_claimed_intake_cannot_promote_or_retarget_after_hold() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    activate_runtime(&db, "runtime-a").unwrap();
    admit_prompt_intake(
        &db,
        NewPromptIntake {
            job_id: "intake",
            target_thread_id: "held-thread",
            channel_id: 10,
            owner_user_id: Some(20),
            discord_message_id: None,
            raw_prompt: "original raw prompt",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: 1.0,
        },
    )
    .unwrap();
    let claim = try_claim_prompt_intake(&db, "intake", 2.0, 600.0)
        .unwrap()
        .unwrap();
    hold(&db);
    let before = get_prompt_intake(&db, "intake").unwrap().unwrap();
    assert!(promote_prompt_intake_to_queue(&db, &claim, job("intake"), 4.0).is_err());
    assert!(canonicalize_prompt_intake_target(&db, "intake").is_err());
    assert!(
        try_claim_prompt_intake(&db, "intake", 700.0, 1300.0)
            .unwrap()
            .is_none()
    );
    assert_eq!(get_prompt_intake(&db, "intake").unwrap().unwrap(), before);
    assert!(list(&db).unwrap().is_empty());
}

#[test]
fn unscoped_request_has_one_safe_startup_notice_without_fabricating_target_hold() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    activate_runtime(&db, "runtime-a").unwrap();
    capture_dead_generation(
        &db,
        DeadGenerationCapture {
            runtime_id: "runtime-a",
            generation: 1,
            snapshot_json: "{\"serverRequests\":[{\"params\":{\"private\":\"preserve-locally\"}}]}",
            affected_targets: &[],
            startup_channel_id: Some(88),
            has_unscoped_requests: true,
            now: 2.0,
        },
    )
    .unwrap();
    let notices = cdr_store::delivery::list_pending(&db).unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].channel_id, 88);
    assert_eq!(notices[0].target_thread_id, "");
    assert!(!notices[0].content.contains("preserve-locally"));
    assert!(!cdr_store::dead_generation::target_is_held(&db, "").unwrap());
}
