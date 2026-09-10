use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use cdr_app_server::ResidentAppServer;
use cdr_runtime::action_executor::{ActionError, ActionExecutor};
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::mapping::upsert_thread;
use rusqlite::Connection;
use tokio::sync::Mutex;

#[path = "support/action_app_server.rs"]
mod action_app_server;
#[path = "support/settings_app_server.rs"]
mod settings_app_server;
use action_app_server::{rpc_log, start_fake_server};

#[derive(Default)]
struct ForkBackend {
    targets: BTreeMap<String, String>,
    forks: Mutex<Vec<String>>,
}

impl TurnBackend for ForkBackend {
    fn generation(&self) -> u64 {
        3
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
        Box::pin(async move {
            self.forks.lock().await.push(thread_id.into());
            self.targets
                .get(thread_id)
                .cloned()
                .ok_or_else(|| BackendFailure::definite(format!("unexpected fork: {thread_id}")))
        })
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
async fn selected_unmapped_settings_preserve_original_target_on_writer_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(start_fake_server(&temp, &log).await);
    let backend = Arc::new(ForkBackend {
        targets: BTreeMap::from([("thread-a".into(), "settings-fork".into())]),
        ..ForkBackend::default()
    });
    let (executor, bridge) = executor(&temp, backend.clone(), server.clone());
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let before = std::fs::read(bridge.path()).unwrap();
    let error = executor
        .execute(
            CommandAction::Settings {
                reference: None,
                model: Some("gpt-5.6-sol".into()),
                effort: Some("max".into()),
                speed: Some("fast".into()),
            },
            10,
            20,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("already has an active writer") && error.contains("no fork was used"));
    assert!(backend.forks.lock().await.is_empty());
    assert_eq!(std::fs::read(bridge.path()).unwrap(), before);
    server.close().await.unwrap();
    let requests = rpc_log(&log);
    assert!(
        requests
            .iter()
            .any(|r| r["method"] == "thread/resume" && r["params"]["threadId"] == "thread-a")
    );
    assert!(!requests.iter().any(|r| matches!(
        r["method"].as_str(),
        Some("thread/fork" | "thread/settings/update" | "turn/start")
    )));
}

#[tokio::test]
async fn stop_and_archive_surface_original_owner_conflict_without_forking() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(start_fake_server(&temp, &log).await);
    let backend = Arc::new(ForkBackend::default());
    let (executor, bridge) = executor(&temp, Arc::clone(&backend), Arc::clone(&server));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();

    let error = executor
        .execute(CommandAction::Stop { reference: None }, 10, 20)
        .await
        .unwrap_err();
    assert!(
        matches!(error, ActionError::Invalid(message) if message.contains("no currently owned active turn is confirmed"))
    );
    assert!(
        !rpc_log(&log)
            .iter()
            .any(|value| value["method"] == "thread/resume"),
        "stop must not acquire ownership merely to guess whether it is idle"
    );
    let error = executor
        .execute(CommandAction::Archive { reference: None }, 10, 20)
        .await
        .unwrap_err();
    assert!(matches!(error, ActionError::Invalid(message) if
        message.contains("requires the app-server that owns original thread thread-a")
            && message.contains("no fork was used")
            && message.contains("already has an active writer")));

    assert!(backend.forks.lock().await.is_empty());
    assert!(
        !rpc_log(&log)
            .iter()
            .any(|value| value["method"] == "thread/archive")
    );
    server.close().await.unwrap();
}

#[tokio::test]
async fn read_only_status_does_not_create_a_fork() {
    let temp = tempfile::tempdir().unwrap();
    let backend = Arc::new(ForkBackend {
        targets: BTreeMap::from([("thread-a".into(), "unexpected".into())]),
        ..ForkBackend::default()
    });
    let state = state_db(&temp);
    let mirror = temp.path().join("mirror.sqlite");
    upsert_thread(&mirror, "thread-a", "project", "Title", 9, 10, 1.0).unwrap();
    let executor = ActionExecutor::new(
        state,
        mirror.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(mirror, Arc::clone(&backend))),
    );

    let status = executor
        .execute(CommandAction::Status { reference: None }, 10, 20)
        .await
        .unwrap();

    assert!(status.text.contains("thread_id: thread-a"));
    assert!(backend.forks.lock().await.is_empty());
}

fn executor(
    temp: &tempfile::TempDir,
    backend: Arc<ForkBackend>,
    server: Arc<ResidentAppServer>,
) -> (ActionExecutor<ForkBackend>, Arc<BridgeState>) {
    let mirror = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let queue = Arc::new(QueueCoordinator::new(mirror.clone(), backend));
    (
        ActionExecutor::new(state_db(temp), mirror, Arc::clone(&bridge), queue).with_server(server),
        bridge,
    )
}

fn state_db(temp: &tempfile::TempDir) -> PathBuf {
    let state = temp.path().join("state.sqlite");
    if !state.exists() {
        Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
    }
    state
}

#[tokio::test]
async fn model_name_is_canonicalized_before_writing_and_options_only_show_names() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(settings_app_server::start(&temp, &log, "normal").await);
    let backend = Arc::new(ForkBackend {
        targets: BTreeMap::from([("thread-b".into(), "settings-fork".into())]),
        ..ForkBackend::default()
    });
    let (executor, bridge) = executor(&temp, backend.clone(), server.clone());
    bridge.set_selected_thread_id(Some("thread-b")).unwrap();
    let result = executor
        .execute(
            CommandAction::Settings {
                reference: None,
                model: Some("MODEL B".into()),
                effort: None,
                speed: None,
            },
            10,
            20,
        )
        .await
        .unwrap();
    assert_eq!(result.text, "모델이 변경되었습니다: model-b");
    assert_eq!(
        bridge.thread_settings("thread-b").unwrap().model.as_deref(),
        Some("model-b")
    );
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("thread-b")
    );
    assert!(backend.forks.lock().await.is_empty());
    let options = executor
        .execute(
            CommandAction::SettingsOptions {
                reference: None,
                field: Some("model".into()),
            },
            10,
            20,
        )
        .await
        .unwrap();
    assert_eq!(options.text, "model-a\nmodel-b");
    server.close().await.unwrap();
    assert!(
        rpc_log(&log)
            .iter()
            .any(|r| r["method"] == "thread/settings/update"
                && r["params"]["threadId"] == "thread-b"
                && r["params"]["model"] == "model-b")
    );
}
