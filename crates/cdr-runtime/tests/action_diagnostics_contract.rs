use std::sync::Arc;

use cdr_runtime::action_executor::ActionExecutor;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use rusqlite::Connection;
#[path = "support/archive_action_guard.rs"]
mod archive_action_guard;
#[path = "support/exact_target_contract.rs"]
mod exact_target;

struct ReadOnlyBackend;

impl TurnBackend for ReadOnlyBackend {
    fn generation(&self) -> u64 {
        1
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

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async { Ok("unused".into()) })
    }
}

#[tokio::test]
async fn status_context_and_mirror_diagnostics_are_real_local_operations() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let mirror = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let queue = Arc::new(QueueCoordinator::new(
        mirror.clone(),
        Arc::new(ReadOnlyBackend),
    ));
    let executor = ActionExecutor::new(state, mirror, bridge, queue);
    executor
        .execute(
            CommandAction::Use {
                reference: "1".into(),
            },
            10,
            20,
        )
        .await
        .unwrap();

    let status = executor
        .execute(CommandAction::Status { reference: None }, 10, 20)
        .await
        .unwrap();
    assert!(status.text.contains("thread-a"));
    assert!(status.text.contains("tokens_used: 100"));
    let context = executor
        .execute(
            CommandAction::Context {
                all_threads: true,
                refresh: false,
                limit: 2,
            },
            10,
            20,
        )
        .await
        .unwrap();
    assert!(context.text.contains("thread-a"));
    assert!(context.text.contains("thread-b"));
    let mirror = executor
        .execute(CommandAction::MirrorCheck, 10, 20)
        .await
        .unwrap_err();
    assert!(mirror.to_string().contains("remote state was not checked"));
}
