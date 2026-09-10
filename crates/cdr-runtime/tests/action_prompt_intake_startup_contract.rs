use std::sync::Arc;

use cdr_runtime::bridge_state::BridgeState;
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, list_prompt_intakes, try_claim_prompt_intake,
};
use cdr_store::queue::{list, mark_app_server_managed_target};

#[path = "support/prompt_intake.rs"]
mod support;
use support::{IntakeBackend, RecordingPreprocessor, executor};

#[tokio::test]
async fn singleton_startup_releases_a_fresh_crashed_claim_and_recovers_immediately() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "managed", 7).unwrap();
    admit_prompt_intake(
        &db,
        NewPromptIntake {
            job_id: "startup-job",
            target_thread_id: "managed",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(903),
            raw_prompt: "survive restart",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: now(),
        },
    )
    .unwrap();
    try_claim_prompt_intake(&db, "startup-job", now(), now() + 600.0)
        .unwrap()
        .unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("managed")).unwrap();
    let backend = Arc::new(IntakeBackend::default());
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );

    assert_eq!(executor.recover_prompt_intakes().await.unwrap(), 0);
    assert!(list(&db).unwrap().is_empty());
    assert_eq!(list_prompt_intakes(&db).unwrap().len(), 1);

    assert_eq!(
        executor.recover_prompt_intakes_on_startup().await.unwrap(),
        1
    );
    assert!(list_prompt_intakes(&db).unwrap().is_empty());
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "startup-job");
    assert_eq!(jobs[0].target_thread_id, "managed");
    assert_eq!(
        backend.starts.lock().await.as_slice(),
        &[("managed".into(), "prepared:managed:survive restart".into())]
    );
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
