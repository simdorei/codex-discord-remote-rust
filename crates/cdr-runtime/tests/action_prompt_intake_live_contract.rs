use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cdr_runtime::action_executor::ActionContext;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::prompt_intake_worker::run_prompt_intake_recovery_worker;
use cdr_runtime::queue_runner::BackendFailure;
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, list_prompt_intakes,
    record_prompt_intake_failure_if_claimed, try_claim_prompt_intake,
};
use cdr_store::queue::{list, mark_app_server_managed_target};
use tokio::sync::watch;

#[path = "support/prompt_intake.rs"]
mod support;
use support::{IntakeBackend, RecordingPreprocessor, executor};

#[tokio::test]
async fn simultaneous_duplicate_message_uses_one_intake_job_fork_and_turn() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("source")).unwrap();
    let backend = Arc::new(IntakeBackend {
        fork_targets: BTreeMap::from([("source".into(), "destination".into())]),
        ..IntakeBackend::default()
    });
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );
    let action = || CommandAction::Interview {
        prompt: "inspect this".into(),
    };

    let (first, second) = tokio::join!(
        executor.execute_with_context(action(), context(701)),
        executor.execute_with_context(action(), context(701)),
    );
    let first = first.unwrap();
    let second = second.unwrap();

    assert!(first.waits_for_final && second.waits_for_final);
    assert_eq!(backend.forks.lock().await.as_slice(), &["source"]);
    assert_eq!(backend.starts.lock().await.len(), 1);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].discord_message_id, Some(701));
    assert_eq!(jobs[0].target_thread_id, "destination");
    assert!(jobs[0].prompt.contains("Gajae-style deep interview"));
    assert!(list_prompt_intakes(&db).unwrap().is_empty());
}

#[tokio::test]
async fn definite_fork_failure_keeps_a_truthful_backed_off_intake_not_an_old_source_job() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("source")).unwrap();
    let backend = Arc::new(IntakeBackend {
        fork_failures: Mutex::new(VecDeque::from([BackendFailure::definite("fork rejected")])),
        ..IntakeBackend::default()
    });
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );

    let error = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "keep this".into(),
            },
            context(702),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("saved for automatic recovery"));
    assert!(error.to_string().contains("fork rejected"));
    assert!(list(&db).unwrap().is_empty());
    assert!(backend.starts.lock().await.is_empty());
    let intake = list_prompt_intakes(&db).unwrap().remove(0);
    assert_eq!(intake.target_thread_id, "source");
    assert_eq!(intake.attempt_count, 1);
    assert!(intake.last_error.contains("fork rejected"));
    assert!(intake.retry_after > now());
}

#[tokio::test]
async fn target_becoming_busy_after_admission_queues_without_a_second_ui_choice() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "managed", 7).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("managed")).unwrap();
    let backend = Arc::new(IntakeBackend {
        become_busy_on_check: Some(2),
        ..IntakeBackend::default()
    });
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );

    let result = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "accepted while idle".into(),
            },
            ActionContext {
                auto_queue_when_busy: false,
                ..context(703)
            },
        )
        .await
        .unwrap();

    assert!(result.ui.is_none());
    assert!(result.waits_for_final);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "managed");
    assert!(jobs[0].turn_id.is_none());
    assert!(list_prompt_intakes(&db).unwrap().is_empty());
    assert!(backend.starts.lock().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn periodic_worker_retries_only_due_intakes_once() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "ready", 7).unwrap();
    mark_app_server_managed_target(&db, "not-due", 7).unwrap();
    admit_failed(&db, "ready-job", "ready", 801, now() - 1.0);
    admit_failed(&db, "later-job", "not-due", 802, now() + 3_600.0);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(IntakeBackend::default());
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );
    let (shutdown, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(run_prompt_intake_recovery_worker(
        Arc::clone(&executor),
        shutdown_rx,
    ));
    tokio::task::yield_now().await;

    tokio::time::advance(Duration::from_secs(29)).await;
    tokio::task::yield_now().await;
    assert!(backend.starts.lock().await.is_empty());

    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        backend.starts.lock().await.as_slice(),
        &[("ready".into(), "prepared:ready:raw-ready-job".into(),)]
    );
    assert_eq!(list_prompt_intakes(&db).unwrap().len(), 1);
    assert_eq!(list_prompt_intakes(&db).unwrap()[0].job_id, "later-job");

    tokio::time::advance(Duration::from_secs(30)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(backend.starts.lock().await.len(), 1);
    shutdown.send(true).unwrap();
    worker.await.unwrap();
}

fn admit_failed(db: &std::path::Path, job: &str, target: &str, message: i64, retry: f64) {
    admit_prompt_intake(
        db,
        NewPromptIntake {
            job_id: job,
            target_thread_id: target,
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(message),
            raw_prompt: &format!("raw-{job}"),
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: now(),
        },
    )
    .unwrap();
    let claim = try_claim_prompt_intake(db, job, now(), now() + 600.0)
        .unwrap()
        .unwrap();
    record_prompt_intake_failure_if_claimed(db, &claim, "transient", retry)
        .unwrap()
        .unwrap();
}

fn context(message_id: u64) -> ActionContext {
    ActionContext {
        channel_id: 99,
        user_id: 20,
        discord_message_id: Some(message_id),
        auto_queue_when_busy: true,
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
