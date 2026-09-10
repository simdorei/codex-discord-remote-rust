use std::path::PathBuf;
use std::sync::Arc;

use cdr_runtime::action_executor::ActionContext;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use cdr_store::dead_generation::{
    DeadGenerationCapture, activate_runtime, capture_dead_generation,
};
use cdr_store::prompt_intake::list_prompt_intakes;
use cdr_store::queue::{list, mark_app_server_managed_target};

#[path = "support/prompt_intake.rs"]
mod support;

struct HoldDuringPreparation(PathBuf);

impl PromptPreprocessor for HoldDuringPreparation {
    fn prepare<'a>(&'a self, prompt: &'a str, thread: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            capture_dead_generation(&self.0, DeadGenerationCapture {
                runtime_id: "intake-hold-test",
                generation: 7,
                snapshot_json: r#"{"generation":7,"closed_reason":"controlled loss","active_turns":[],"server_requests":[]}"#,
                affected_targets: &[thread.to_owned()],
                startup_channel_id: Some(99),
                has_unscoped_requests: false,
                now: 100.0,
            }).unwrap();
            Ok(prompt.to_owned())
        })
    }
}

#[tokio::test]
async fn concurrent_manual_hold_preserves_intake_without_promising_automatic_retry() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    activate_runtime(&db, "intake-hold-test").unwrap();
    mark_app_server_managed_target(&db, "managed", 7).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("managed")).unwrap();
    let backend = Arc::new(support::IntakeBackend::default());
    let (_, executor) = support::executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        Arc::new(HoldDuringPreparation(db.clone())),
    );
    let error = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "preserve my exact request".into(),
            },
            ActionContext {
                channel_id: 99,
                user_id: 20,
                discord_message_id: Some(901),
                auto_queue_when_busy: true,
            },
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(!error.contains("saved for automatic recovery"), "{error}");
    assert!(error.contains("manual hold"), "{error}");
    assert!(
        error.contains("will not be retried automatically"),
        "{error}"
    );
    assert!(
        error.contains("manual review is required"),
        "real cause missing: {error}"
    );
    let saved = list_prompt_intakes(&db).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].raw_prompt, "preserve my exact request");
    assert!(error.contains(&saved[0].job_id));
    assert!(list(&db).unwrap().is_empty());
    assert!(backend.starts.lock().await.is_empty());
    assert_eq!(
        executor.recover_prompt_intakes_on_startup().await.unwrap(),
        0
    );
    assert_eq!(list_prompt_intakes(&db).unwrap().len(), 1);
    assert!(backend.starts.lock().await.is_empty());
}
