use cdr_runtime::action_executor::{ActionContext, ActionError, ActionExecutor};
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{QueueJobState, list};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default)]
struct FakeBackend {
    active: Mutex<Option<String>>,
    starts: Mutex<Vec<String>>,
}

#[tokio::test]
async fn help_explains_new_and_preserves_archive_and_control_commands() {
    let temp = tempfile::tempdir().unwrap();
    let (executor, _) = executor(&temp);
    let result = executor.execute(CommandAction::Help, 99, 20).await.unwrap();
    for command in [
        "!new <요청>",
        "!archive",
        "!archived_list",
        "!resume",
        "!mirror sync",
        "!steer",
        "!stop",
        "!settings",
        "!usage",
    ] {
        assert!(result.text.contains(command), "help missing {command}");
    }
    assert!(result.text.contains("새 Codex"));
}

impl TurnBackend for FakeBackend {
    fn generation(&self) -> u64 {
        7
    }

    fn requires_app_server_fork(&self) -> bool {
        true
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move { Ok(self.active.lock().await.clone()) })
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
            *self.active.lock().await = Some("turn-1".into());
            Ok("turn-1".into())
        })
    }
}

fn state_db(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
}

fn executor(temp: &tempfile::TempDir) -> (ActionExecutor<FakeBackend>, Arc<FakeBackend>) {
    let state = temp.path().join("state.sqlite");
    state_db(&state);
    let mirror = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(FakeBackend::default());
    let queue = Arc::new(QueueCoordinator::new(mirror.clone(), Arc::clone(&backend)));
    (ActionExecutor::new(state, mirror, bridge, queue), backend)
}

#[tokio::test]
async fn mirror_sync_cannot_report_success_without_a_discord_sync_transport() {
    let temp = tempfile::tempdir().unwrap();
    let (executor, _) = executor(&temp);
    let result = executor
        .execute(CommandAction::BridgeSync { limit: None }, 10, 20)
        .await;
    assert!(
        result.is_err(),
        "a status-only response is not a completed mirror sync"
    );
}

#[tokio::test]
async fn list_use_and_where_share_the_python_reference_and_selected_state() {
    let temp = tempfile::tempdir().unwrap();
    let (executor, _) = executor(&temp);
    let listed = executor
        .execute(CommandAction::List { limit: 10 }, 10, 20)
        .await
        .unwrap();
    assert!(listed.text.contains("1 | alpha"));
    assert!(listed.text.contains("thread-a"));

    let used = executor
        .execute(
            CommandAction::Use {
                reference: "2".into(),
            },
            10,
            20,
        )
        .await
        .unwrap();
    assert!(used.text.contains("thread-b"));
    let current = executor
        .execute(CommandAction::Where, 10, 20)
        .await
        .unwrap();
    assert!(current.text.contains("selected"));
    assert!(current.text.contains("thread-b"));
}

#[tokio::test]
async fn mirrored_channel_wins_over_selected_target_and_ask_uses_durable_queue() {
    let temp = tempfile::tempdir().unwrap();
    let (executor, backend) = executor(&temp);
    executor
        .execute(
            CommandAction::Use {
                reference: "2".into(),
            },
            10,
            20,
        )
        .await
        .unwrap();
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
    let result = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "hello".into(),
            },
            ActionContext {
                channel_id: 10,
                user_id: 20,
                discord_message_id: Some(30),
                auto_queue_when_busy: false,
            },
        )
        .await
        .unwrap();

    assert!(result.waits_for_final);
    assert_eq!(result.text, "In progress\nmessage: hello");
    assert_eq!(*backend.starts.lock().await, vec!["hello"]);
    let jobs = list(executor.mirror_db()).unwrap();
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(jobs[0].target_thread_id, "thread-a-bot");
    assert_eq!(jobs[0].discord_message_id, Some(30));
}

#[tokio::test]
async fn no_target_and_unsupported_actions_surface_actual_errors() {
    let temp = tempfile::tempdir().unwrap();
    let (executor, _) = executor(&temp);
    assert!(matches!(
        executor
            .execute(
                CommandAction::Ask {
                    prompt: "hello".into()
                },
                99,
                20
            )
            .await,
        Err(ActionError::NoTarget)
    ));
    assert!(
        executor
            .execute(CommandAction::QaButtons, 10, 20)
            .await
            .unwrap()
            .text
            .contains("button QA")
    );
}

#[tokio::test]
async fn archived_list_and_retract_are_real_store_operations() {
    let temp = tempfile::tempdir().unwrap();
    let (executor, backend) = executor(&temp);
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
    *backend.active.lock().await = Some("busy".into());
    executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "later".into(),
            },
            ActionContext {
                channel_id: 10,
                user_id: 20,
                discord_message_id: None,
                auto_queue_when_busy: true,
            },
        )
        .await
        .unwrap();
    let archived = executor
        .execute(CommandAction::ArchivedList { limit: 10 }, 10, 20)
        .await
        .unwrap();
    assert!(archived.text.contains("thread-old"));
    let retracted = executor
        .execute(CommandAction::Retract { reference: None }, 10, 20)
        .await
        .unwrap();
    assert!(retracted.text.contains("Retracted"));
    assert!(list(executor.mirror_db()).unwrap().is_empty());
}
