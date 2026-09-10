use std::collections::VecDeque;
use std::sync::Arc;

use cdr_runtime::action_executor::{ActionContext, ActionExecutor};
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::queue::{QueueJobState, list};
use rusqlite::Connection;
use tokio::sync::Mutex;

struct FailingBackend {
    failures: Mutex<VecDeque<BackendFailure>>,
    starts: Mutex<usize>,
}

impl TurnBackend for FailingBackend {
    fn generation(&self) -> u64 {
        7
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
        Box::pin(async move {
            *self.starts.lock().await += 1;
            Err(self.failures.lock().await.pop_front().unwrap())
        })
    }
}

#[tokio::test]
async fn accepted_backend_failure_is_a_visible_action_result_and_duplicate_is_not_redriven() {
    for failure in [
        BackendFailure::definite("transport failed: exact definite warning"),
        BackendFailure::ambiguous("transport failed: exact ambiguous warning"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state.sqlite");
        Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        let mirror = temp.path().join("mirror.sqlite");
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        bridge.set_selected_thread_id(Some("thread-a")).unwrap();
        let backend = Arc::new(FailingBackend {
            failures: Mutex::new(VecDeque::from([failure.clone()])),
            starts: Mutex::new(0),
        });
        let queue = Arc::new(QueueCoordinator::new(mirror.clone(), Arc::clone(&backend)));
        let executor = ActionExecutor::new(state, mirror, bridge, queue);
        let context = ActionContext {
            channel_id: 10,
            user_id: 20,
            discord_message_id: Some(30),
            auto_queue_when_busy: false,
        };

        let first = executor
            .execute_with_context(
                CommandAction::Ask {
                    prompt: "keep this".into(),
                },
                context,
            )
            .await
            .unwrap();
        let duplicate = executor
            .execute_with_context(
                CommandAction::Ask {
                    prompt: "different duplicate body".into(),
                },
                context,
            )
            .await
            .unwrap();

        assert_eq!(duplicate, first);
        assert!(first.waits_for_final);
        assert!(first.text.contains("Accepted"));
        if failure.ambiguous {
            assert!(first.text.contains("outcome is unknown"));
            assert!(first.text.contains("recovery will reconcile"));
            assert!(!first.text.contains("queued for automatic retry"));
        } else {
            assert!(first.text.contains("queued for automatic retry"));
        }
        assert!(first.text.contains(if failure.ambiguous {
            "ambiguous"
        } else {
            "definite"
        }));
        assert!(first.text.contains(&failure.message));
        assert_eq!(*backend.starts.lock().await, 1);
        let jobs = list(executor.mirror_db()).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].prompt, "keep this");
        assert_eq!(
            jobs[0].state,
            if failure.ambiguous {
                QueueJobState::Starting
            } else {
                QueueJobState::Pending
            }
        );
    }
}
