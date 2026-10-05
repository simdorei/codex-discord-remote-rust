use cdr_runtime::{
    action_executor::{ActionContext, ActionExecutor},
    bridge_state::BridgeState,
    command_plan::CommandAction,
    queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord},
};
use cdr_store::ingress::{self, IngressKind, NewIngress};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

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

fn context(event: u64, channel: u64) -> ActionContext {
    ActionContext {
        channel_id: channel,
        user_id: 20,
        discord_message_id: Some(event),
        auto_queue_when_busy: true,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unowned_stop_does_not_wait_for_target_lock_or_need_an_app_server() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "A", 98, 99, 1.0).unwrap();
    cdr_store::mapping::upsert_thread(&db, "thread-a", "project", "B", 98, 100, 1.0).unwrap();
    ingress::admit(
        &db,
        &NewIngress {
            ingress_id: "message:701".into(),
            kind: IngressKind::Message,
            event_id: Some(701),
            application_id: None,
            channel_id: 99,
            owner_user_id: 20,
            source_message_id: Some(701),
            payload: json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"original A"}}}}),
            target_thread_id: Some("thread-b".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    ingress::begin_execution(&db, "message:701", "processing", Some("thread-b"), 2.0).unwrap();
    let before = ingress::get(&db, "message:701").unwrap().unwrap();
    let backend = Arc::new(Backend::default());
    let queue = Arc::new(QueueCoordinator::new(db.clone(), Arc::clone(&backend)));
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        queue,
    );
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
    let stopped = stopped
        .expect("stop must not wait on the target lock")
        .expect("durable unowned stop must not need an app server");
    assert!(stopped.text.contains("Stop accepted"));
    assert!(stopped.text.contains("Unowned original requests held: 1"));
    assert!(stopped.text.contains("Execution end is not confirmed"));
    assert!(!stopped.waits_for_final);
    let saved = ingress::get(&db, "message:701").unwrap().unwrap();
    assert_eq!(saved.payload, before.payload);
    assert_eq!(saved.state, "held");
    assert_eq!(saved.owner_id, None);

    assert!(
        executor
            .execute_with_context(
                CommandAction::Ask {
                    prompt: "original A".into()
                },
                context(701, 99),
            )
            .await
            .is_err()
    );
    assert!(backend.starts.lock().await.is_empty());
    assert!(
        cdr_store::prompt_intake::list_prompt_intakes(&db)
            .unwrap()
            .is_empty()
    );
    assert!(
        cdr_store::queue::list_filtered(&db, None, None)
            .unwrap()
            .is_empty()
    );
    let independent = tokio::time::timeout(
        Duration::from_secs(5),
        executor.execute_with_context(
            CommandAction::Ask {
                prompt: "independent B".into(),
            },
            context(702, 100),
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
}
