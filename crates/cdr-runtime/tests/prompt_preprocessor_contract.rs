use std::sync::Arc;

use cdr_runtime::action_executor::ActionExecutor;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use cdr_store::mapping::upsert_thread;
use rusqlite::Connection;
use tokio::sync::Mutex;

#[derive(Default)]
struct FakeBackend {
    starts: Mutex<Vec<String>>,
}

impl TurnBackend for FakeBackend {
    fn generation(&self) -> u64 {
        1
    }

    fn requires_app_server_fork(&self) -> bool {
        true
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn fork_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move { Ok(format!("{thread_id}-bot")) })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts.lock().await.push(prompt.into());
            Ok("turn-1".into())
        })
    }
}

struct PrefixPreprocessor;

impl PromptPreprocessor for PrefixPreprocessor {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move { Ok(format!("prepared:{thread_id}:{prompt}")) })
    }
}

#[tokio::test]
async fn preprocessor_runs_after_target_resolution_and_before_queueing() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let mirror = temp.path().join("mirror.sqlite");
    let backend = Arc::new(FakeBackend::default());
    let queue = Arc::new(QueueCoordinator::new(mirror.clone(), Arc::clone(&backend)));
    let executor = ActionExecutor::new(
        state,
        mirror,
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        queue,
    )
    .with_prompt_preprocessor(Arc::new(PrefixPreprocessor));
    upsert_thread(
        executor.mirror_db(),
        "thread-a",
        "project",
        "Alpha",
        9,
        10,
        1.0,
    )
    .unwrap();

    executor
        .execute(
            CommandAction::Ask {
                prompt: "hello".into(),
            },
            10,
            20,
        )
        .await
        .unwrap();

    assert_eq!(
        *backend.starts.lock().await,
        vec!["prepared:thread-a-bot:hello"]
    );
}
