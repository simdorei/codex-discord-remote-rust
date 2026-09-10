use std::sync::Arc;
use std::time::Duration;

use cdr_runtime::action_executor::{ActionContext, ActionResult};
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use cdr_store::prompt_intake::{get_prompt_intake, list_prompt_intakes, try_claim_prompt_intake};
use cdr_store::queue::{list, mark_app_server_managed_target};
use rusqlite::Connection;
use tokio::sync::Semaphore;

#[path = "support/prompt_intake.rs"]
mod support;
use support::{IntakeBackend, RecordingPreprocessor, executor};

#[tokio::test]
async fn claim_lost_before_processing_has_no_preprocess_fork_queue_or_start_side_effect() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "managed", 7).unwrap();
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER steal_prompt_intake_claim
             AFTER UPDATE OF claim_token ON codex_prompt_intakes
             WHEN NEW.claim_token IS NOT NULL AND NEW.claim_token <> 'stolen-by-test'
             BEGIN
               UPDATE codex_prompt_intakes
               SET claim_token = 'stolen-by-test'
               WHERE job_id = NEW.job_id;
             END;",
        )
        .unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("managed")).unwrap();
    let backend = Arc::new(IntakeBackend::default());
    let preprocessor = Arc::new(RecordingPreprocessor::new());
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        preprocessor.clone(),
    );

    let result = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "must remain durable".into(),
            },
            context(902),
        )
        .await
        .unwrap();

    assert!(
        result
            .text
            .contains("durable preparation or retry is already pending")
    );
    assert!(preprocessor.seen.lock().unwrap().is_empty());
    assert!(backend.forks.lock().await.is_empty());
    assert!(list(&db).unwrap().is_empty());
    assert!(backend.starts.lock().await.is_empty());
    assert_eq!(list_prompt_intakes(&db).unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn long_preprocessing_renews_ownership_and_prevents_an_overlapping_processor() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "managed", 7).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("managed")).unwrap();
    let backend = Arc::new(IntakeBackend::default());
    let preprocessor = Arc::new(SlowPreprocessor::new());
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        preprocessor.clone(),
    );
    let running = tokio::spawn({
        let executor = Arc::clone(&executor);
        async move {
            executor
                .execute_with_context(
                    CommandAction::Ask {
                        prompt: "slow but owned".into(),
                    },
                    context(901),
                )
                .await
        }
    });

    preprocessor.entered.acquire().await.unwrap().forget();
    let intake = list_prompt_intakes(&db).unwrap().remove(0);
    let before = now();
    Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE codex_prompt_intakes SET claim_expires_at = ? WHERE job_id = ?",
            rusqlite::params![before + 60.0, intake.job_id],
        )
        .unwrap();

    tokio::time::advance(Duration::from_mins(2)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    let renewed = get_prompt_intake(&db, &intake.job_id).unwrap().unwrap();
    assert!(renewed.claim_expires_at > before + 500.0);
    assert!(
        try_claim_prompt_intake(&db, &intake.job_id, before + 61.0, before + 661.0)
            .unwrap()
            .is_none()
    );

    preprocessor.release.add_permits(1);
    let result: ActionResult = running.await.unwrap().unwrap();
    assert_eq!(result.text, "In progress\nmessage: slow but owned");
    assert!(list_prompt_intakes(&db).unwrap().is_empty());
    assert_eq!(list(&db).unwrap().len(), 1);
    assert_eq!(list(&db).unwrap()[0].job_id, intake.job_id);
    assert_eq!(backend.starts.lock().await.len(), 1);
}

struct SlowPreprocessor {
    entered: Semaphore,
    release: Semaphore,
}

impl SlowPreprocessor {
    fn new() -> Self {
        Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        }
    }
}

impl PromptPreprocessor for SlowPreprocessor {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            Ok(format!("prepared:{thread_id}:{prompt}"))
        })
    }
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
