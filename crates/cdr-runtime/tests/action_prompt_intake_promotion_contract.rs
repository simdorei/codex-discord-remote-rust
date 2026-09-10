use std::sync::Arc;

use cdr_runtime::action_executor::ActionContext;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_store::delivery::list_pending;
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, get_prompt_intake, list_prompt_intakes,
    try_claim_prompt_intake,
};
use cdr_store::queue::{list, mark_app_server_managed_target};

#[path = "support/prompt_intake.rs"]
mod support;
use support::{IntakeBackend, RecordingPreprocessor, executor};

#[tokio::test]
async fn completion_after_runtime_promotion_leaves_no_intake_for_restart_to_replay() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "managed", 7).unwrap();
    let claim = admit_and_claim(&db, "stable-job", 901);
    let backend = Arc::new(IntakeBackend::default());
    let (queue, executor) = executor(
        &temp,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );

    // This is the ActionExecutor's inner submit boundary: promotion has committed,
    // but no action response has been constructed yet.
    let submission = queue
        .submit_prompt_intake(&claim, "managed", "prepared:managed:raw")
        .await
        .unwrap();
    let turn_id = submission.turn_id.expect("the promoted job starts once");
    assert!(get_prompt_intake(&db, "stable-job").unwrap().is_none());
    assert_eq!(backend.starts.lock().await.len(), 1);

    queue
        .stage_turn_completion("managed", &turn_id, "one final answer")
        .await
        .unwrap()
        .expect("real completion is staged");
    assert!(list(&db).unwrap().is_empty());
    assert_eq!(list_pending(&db).unwrap().len(), 1);

    assert_eq!(executor.recover_prompt_intakes().await.unwrap(), 0);
    assert_eq!(queue.recover().await.unwrap().started, 0);
    assert_eq!(backend.starts.lock().await.len(), 1);
    assert!(list_prompt_intakes(&db).unwrap().is_empty());
    assert_eq!(list_pending(&db).unwrap().len(), 1);
}

#[tokio::test]
async fn live_action_uses_atomic_promotion_and_keeps_the_final_preprocessed_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "managed", 7).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("managed")).unwrap();
    let backend = Arc::new(IntakeBackend::default());
    let preprocessor = Arc::new(RecordingPreprocessor::new());
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        preprocessor,
    );

    let result = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "raw attachment prompt".into(),
            },
            ActionContext {
                channel_id: 99,
                user_id: 20,
                discord_message_id: Some(902),
                auto_queue_when_busy: true,
            },
        )
        .await
        .unwrap();

    assert!(result.waits_for_final);
    assert!(list_prompt_intakes(&db).unwrap().is_empty());
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].prompt, "prepared:managed:raw attachment prompt");
    assert_eq!(backend.starts.lock().await.len(), 1);
}

fn admit_and_claim(
    db: &std::path::Path,
    job_id: &str,
    message_id: i64,
) -> cdr_store::prompt_intake::PromptIntakeClaim {
    let now = now();
    admit_prompt_intake(
        db,
        NewPromptIntake {
            job_id,
            target_thread_id: "managed",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(message_id),
            raw_prompt: "raw",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: now,
        },
    )
    .unwrap();
    try_claim_prompt_intake(db, job_id, now, now + 600.0)
        .unwrap()
        .unwrap()
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
