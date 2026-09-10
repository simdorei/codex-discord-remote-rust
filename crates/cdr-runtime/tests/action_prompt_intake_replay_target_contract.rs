use std::collections::BTreeMap;
use std::sync::Arc;

use cdr_runtime::action_executor::{ActionContext, ActionResult};
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use cdr_store::prompt_intake::list_prompt_intakes;
use cdr_store::queue::list;
use tokio::sync::Semaphore;

#[path = "support/prompt_intake.rs"]
mod support;
use support::{IntakeBackend, executor};

#[tokio::test]
async fn duplicate_during_final_target_preprocessing_reports_the_final_target() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("source")).unwrap();
    let backend = Arc::new(IntakeBackend {
        fork_targets: BTreeMap::from([("source".into(), "destination".into())]),
        ..IntakeBackend::default()
    });
    let preprocessor = Arc::new(GatedPreprocessor::new());
    let (_, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        preprocessor.clone(),
    );
    let first = tokio::spawn({
        let executor = Arc::clone(&executor);
        async move { executor.execute_with_context(action(), context()).await }
    });
    preprocessor.entered.acquire().await.unwrap().forget();

    let duplicate = executor
        .execute_with_context(action(), context())
        .await
        .unwrap();

    assert!(duplicate.text.contains("thread_id: destination"));
    assert!(!duplicate.text.contains("thread_id: source"));
    assert!(list(&db).unwrap().is_empty());
    assert_eq!(
        list_prompt_intakes(&db).unwrap()[0].target_thread_id,
        "destination"
    );
    preprocessor.release.add_permits(1);
    let completed: ActionResult = first.await.unwrap().unwrap();
    assert_eq!(
        completed.text,
        "In progress\nmessage: same accepted request"
    );
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(job_id(&duplicate.text), jobs[0].job_id);
    assert_eq!(backend.forks.lock().await.as_slice(), &["source"]);
    assert_eq!(backend.starts.lock().await.len(), 1);
}

struct GatedPreprocessor {
    entered: Semaphore,
    release: Semaphore,
}

impl GatedPreprocessor {
    fn new() -> Self {
        Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        }
    }
}

impl PromptPreprocessor for GatedPreprocessor {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            Ok(format!("prepared:{thread_id}:{prompt}"))
        })
    }
}

fn action() -> CommandAction {
    CommandAction::Ask {
        prompt: "same accepted request".into(),
    }
}

fn context() -> ActionContext {
    ActionContext {
        channel_id: 99,
        user_id: 20,
        discord_message_id: Some(904),
        auto_queue_when_busy: true,
    }
}

fn job_id(text: &str) -> &str {
    text.lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .unwrap()
}
