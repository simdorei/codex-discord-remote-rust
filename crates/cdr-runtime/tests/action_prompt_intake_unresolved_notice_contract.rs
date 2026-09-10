use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cdr_runtime::action_executor::ActionContext;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::prompt_intake_worker::run_prompt_intake_recovery_worker;
use cdr_runtime::queue_runner::BackendFailure;
use cdr_store::delivery::{complete as complete_delivery, list_pending};
use cdr_store::prompt_intake::list_prompt_intakes;
use cdr_store::queue::{UNRESOLVED_FORK_ERROR_PREFIX, list};
use cdr_store::schema::open_initialized;
use tokio::sync::watch;

#[path = "support/prompt_intake.rs"]
mod support;
use support::{IntakeBackend, RecordingPreprocessor, executor};

#[tokio::test(start_paused = true)]
async fn live_startup_and_periodic_recovery_keep_one_delivered_fork_notice_suppressed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("source")).unwrap();
    let backend = Arc::new(IntakeBackend {
        fork_failures: Mutex::new(VecDeque::from([BackendFailure::ambiguous(
            "fork transport timed out",
        )])),
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
                prompt: "must remain durable".into(),
            },
            ActionContext {
                channel_id: 99,
                user_id: 20,
                discord_message_id: Some(990),
                auto_queue_when_busy: true,
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("saved for automatic recovery"));
    assert!(list(&db).unwrap().is_empty());
    assert!(backend.starts.lock().await.is_empty());
    assert_eq!(backend.forks.lock().await.as_slice(), &["source"]);
    backend.forks.lock().await.clear();
    assert_fenced_intake(&db);
    let notice = list_pending(&db).unwrap().pop().unwrap();
    assert!(notice.delivery_id.starts_with("fork-unresolved-intake:"));
    assert_eq!(notice.delivery_id, notice.job_id);
    assert!(complete_delivery(&db, &notice.delivery_id).unwrap());

    make_due(&db);
    assert_eq!(
        executor.recover_prompt_intakes_on_startup().await.unwrap(),
        0
    );
    assert_recovery_did_not_start(&db, &backend).await;

    make_due(&db);
    let (shutdown, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(run_prompt_intake_recovery_worker(
        Arc::clone(&executor),
        shutdown_rx,
    ));
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(30)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_recovery_did_not_start(&db, &backend).await;
    shutdown.send(true).unwrap();
    worker.await.unwrap();
}

fn assert_fenced_intake(db: &std::path::Path) {
    let intakes = list_prompt_intakes(db).unwrap();
    assert_eq!(intakes.len(), 1);
    assert!(
        intakes[0]
            .last_error
            .starts_with(UNRESOLVED_FORK_ERROR_PREFIX)
    );
    assert_eq!(
        intakes[0]
            .last_error
            .matches(UNRESOLVED_FORK_ERROR_PREFIX)
            .count(),
        1
    );
    assert!(intakes[0].last_error.contains("fork transport timed out"));
}

async fn assert_recovery_did_not_start(db: &std::path::Path, backend: &IntakeBackend) {
    assert!(list(db).unwrap().is_empty());
    assert_fenced_intake(db);
    assert!(
        list_pending(db).unwrap().is_empty(),
        "a delivered notice must not be recreated"
    );
    assert!(backend.forks.lock().await.is_empty());
    assert!(backend.starts.lock().await.is_empty());
}

fn make_due(db: &std::path::Path) {
    open_initialized(db)
        .unwrap()
        .execute(
            "UPDATE codex_prompt_intakes SET retry_after = 0, claim_token = NULL, \
             claim_expires_at = 0",
            [],
        )
        .unwrap();
}
