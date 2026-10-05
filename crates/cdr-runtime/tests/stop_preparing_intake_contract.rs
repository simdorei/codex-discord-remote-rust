use std::{sync::Arc, time::Duration};

use cdr_runtime::{
    action_executor::{ActionContext, ActionExecutor},
    bridge_state::BridgeState,
    command_plan::CommandAction,
    prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor},
    queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord},
};
use tokio::sync::{Mutex, Semaphore};

#[path = "support/action_app_server.rs"]
mod server_support;

#[derive(Default)]
struct Backend {
    starts: Mutex<Vec<(String, String)>>,
}

impl TurnBackend for Backend {
    fn generation(&self) -> u64 {
        7
    }
    fn active_turn_id<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }
    fn read_turns<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn resume_thread<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn start_turn<'a>(&'a self, thread: &'a str, prompt: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts
                .lock()
                .await
                .push((thread.into(), prompt.into()));
            Ok(format!("turn-{thread}"))
        })
    }
}

struct PausedPreparation {
    entered: Semaphore,
    release: Semaphore,
}

impl PromptPreprocessor for PausedPreparation {
    fn prepare<'a>(&'a self, prompt: &'a str, target: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            if target == "thread-b" {
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
            Ok(prompt.into())
        })
    }
}

fn context() -> ActionContext {
    ActionContext {
        channel_id: 99,
        user_id: 20,
        discord_message_id: Some(701),
        auto_queue_when_busy: true,
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    db: std::path::PathBuf,
    server: Arc<cdr_app_server::ResidentAppServer>,
    backend: Arc<Backend>,
    executor: Arc<ActionExecutor<Backend>>,
    preparation: Arc<PausedPreparation>,
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "A", 98, 99, 1.0).unwrap();
    cdr_store::mapping::upsert_thread(&db, "thread-a", "project", "B", 98, 100, 1.0).unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(server_support::start_fake_server(&temp, &log).await);
    let backend = Arc::new(Backend::default());
    let queue = Arc::new(QueueCoordinator::new(db.clone(), Arc::clone(&backend)));
    let preparation = Arc::new(PausedPreparation {
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let executor = Arc::new(
        ActionExecutor::new(
            state,
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            queue,
        )
        .with_server(Arc::clone(&server))
        .with_prompt_preprocessor(preparation.clone()),
    );
    Fixture {
        _temp: temp,
        db,
        server,
        backend,
        executor,
        preparation,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_during_preparation_is_bounded_and_original_never_starts_or_promises_retry() {
    let Fixture {
        _temp: temp,
        db,
        server,
        backend,
        executor,
        preparation,
    } = fixture().await;
    let log = temp.path().join("rpc.jsonl");
    let first_executor = Arc::clone(&executor);
    let first = tokio::spawn(async move {
        first_executor
            .execute_with_context(
                CommandAction::Ask {
                    prompt: "original A".into(),
                },
                context(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), preparation.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let before = cdr_store::prompt_intake::list_prompt_intakes(&db).unwrap();
    assert_eq!(before.len(), 1);
    assert!(before[0].claim_token.is_some());
    assert!(
        cdr_store::queue::list_filtered(&db, None, None)
            .unwrap()
            .is_empty()
    );

    // A normal stop must not wait for the slow target/control task to complete.
    let guard = executor.control_lock("thread-b").await.unwrap();
    let stopped = tokio::time::timeout(
        Duration::from_secs(3),
        executor.execute(
            CommandAction::Stop {
                reference: Some("thread-b".into()),
            },
            99,
            20,
        ),
    )
    .await;
    drop(guard);
    preparation.release.add_permits(1);
    let completed = tokio::time::timeout(Duration::from_secs(5), first)
        .await
        .unwrap()
        .unwrap();
    if stopped.is_err() || stopped.as_ref().is_ok_and(Result::is_err) {
        server.close().await.unwrap();
        panic!("preparing stop was not accepted before releasing target lock: {stopped:?}");
    }
    let stopped = stopped.unwrap().unwrap();
    assert!(stopped.text.contains("Stop accepted"));
    assert!(stopped.text.contains("preparing"));
    assert!(stopped.text.contains("Execution end is not confirmed"));
    assert!(!stopped.waits_for_final);
    let error = completed.unwrap_err().to_string();
    assert!(
        error.contains("will not be retried automatically"),
        "{error}"
    );
    assert_eq!(
        cdr_store::prompt_intake::list_prompt_intakes(&db).unwrap(),
        before
    );
    assert!(backend.starts.lock().await.is_empty());

    let replay = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "original A".into(),
            },
            context(),
        )
        .await
        .unwrap();
    assert!(
        replay.text.contains("will not be retried automatically"),
        "{}",
        replay.text
    );
    assert!(!replay.waits_for_final);
    assert_eq!(
        cdr_store::prompt_intake::list_prompt_intakes(&db).unwrap(),
        before
    );

    assert_progress_and_close(&executor, &backend, &server, &log).await;
}

async fn assert_progress_and_close(
    executor: &ActionExecutor<Backend>,
    backend: &Backend,
    server: &cdr_app_server::ResidentAppServer,
    log: &std::path::Path,
) {
    let independent = tokio::time::timeout(
        Duration::from_secs(5),
        executor.execute_with_context(
            CommandAction::Ask {
                prompt: "independent B".into(),
            },
            ActionContext {
                channel_id: 100,
                user_id: 21,
                discord_message_id: Some(702),
                auto_queue_when_busy: true,
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(independent.waits_for_final);
    assert_eq!(
        *backend.starts.lock().await,
        [("thread-a".into(), "independent B".into())]
    );
    assert!(server_support::rpc_log(log).iter().all(|call| !matches!(
        call["method"].as_str(),
        Some("turn/start" | "turn/interrupt" | "thread/resume" | "thread/fork")
    )));
    tokio::time::timeout(Duration::from_secs(3), server.close())
        .await
        .unwrap()
        .unwrap();
}
