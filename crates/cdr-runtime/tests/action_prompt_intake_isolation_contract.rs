use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::queue_runner::BackendFailure;
use cdr_store::prompt_intake::{NewPromptIntake, admit_prompt_intake, list_prompt_intakes};
use cdr_store::queue::list;

#[path = "support/prompt_intake.rs"]
mod support;
use support::{IntakeBackend, RecordingPreprocessor, executor};

#[tokio::test]
async fn one_failed_intake_is_backed_off_without_starving_the_next_ready_row() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    admit(&db, "job-a", "source-a", 1001, 1.0);
    admit(&db, "job-b", "source-b", 1002, 2.0);
    let backend = Arc::new(IntakeBackend {
        fork_targets: BTreeMap::from([("source-b".into(), "target-b".into())]),
        fork_failures: Mutex::new(VecDeque::from([BackendFailure::definite(
            "source-a unavailable",
        )])),
        ..IntakeBackend::default()
    });
    let (_, executor) = executor(
        &temp,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );

    assert_eq!(executor.recover_prompt_intakes().await.unwrap(), 1);

    let intakes = list_prompt_intakes(&db).unwrap();
    assert_eq!(intakes.len(), 1);
    assert_eq!(intakes[0].job_id, "job-a");
    assert_eq!(intakes[0].attempt_count, 1);
    assert!(intakes[0].last_error.contains("source-a unavailable"));
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "job-b");
    assert_eq!(jobs[0].target_thread_id, "target-b");
    assert_eq!(backend.starts.lock().await.len(), 1);
}

fn admit(db: &std::path::Path, job: &str, target: &str, message: i64, created_at: f64) {
    admit_prompt_intake(
        db,
        NewPromptIntake {
            job_id: job,
            target_thread_id: target,
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(message),
            raw_prompt: job,
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at,
        },
    )
    .unwrap();
}
