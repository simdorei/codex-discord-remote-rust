use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cdr_runtime::action_executor::{ActionContext, ActionError};
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use cdr_store::mapping::upsert_thread;
use cdr_store::prompt_intake::list_prompt_intakes;
use cdr_store::queue::{
    NewAppServerForkHandoff, begin_app_server_fork_handoff, complete_app_server_fork_handoff, list,
    mark_app_server_managed_target,
};
use rusqlite::Connection;

#[path = "support/action_target.rs"]
mod action_target;
use action_target::{FakeBackend, executor};

#[tokio::test]
async fn race_reprepares_for_the_new_target_before_atomic_enqueue() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "thread-a", "project", "Main", 9, 10, 1.0).unwrap();
    let backend = Arc::new(FakeBackend {
        fork_targets: BTreeMap::from([("thread-a".into(), "fork-1".into())]),
        ..FakeBackend::default()
    });
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let preprocessor = Arc::new(MappingRacePreprocessor::new(
        db.clone(),
        [("race", "fork-1", "fork-2")],
    ));
    let executor = executor(&temp, db.clone(), Arc::clone(&bridge), backend)
        .with_prompt_preprocessor(preprocessor.clone());

    executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "hello".into(),
            },
            ActionContext {
                channel_id: 10,
                user_id: 20,
                discord_message_id: Some(40),
                auto_queue_when_busy: true,
            },
        )
        .await
        .unwrap();

    assert_eq!(*preprocessor.seen.lock().unwrap(), vec!["fork-1", "fork-2"]);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "fork-2");
    assert_eq!(jobs[0].prompt, "prepared:fork-2:hello");
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("fork-2")
    );
}

#[tokio::test]
async fn second_race_fails_closed_without_enqueuing() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "thread-a", "project", "Main", 9, 10, 1.0).unwrap();
    let backend = Arc::new(FakeBackend {
        fork_targets: BTreeMap::from([("thread-a".into(), "fork-1".into())]),
        ..FakeBackend::default()
    });
    let preprocessor = Arc::new(MappingRacePreprocessor::new(
        db.clone(),
        [
            ("race-1", "fork-1", "fork-2"),
            ("race-2", "fork-2", "fork-3"),
        ],
    ));
    let executor = executor(
        &temp,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        backend,
    )
    .with_prompt_preprocessor(preprocessor);

    let error = executor
        .execute(
            CommandAction::Ask {
                prompt: "hello".into(),
            },
            10,
            20,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, ActionError::Invalid(message) if
        message.contains("mapping changed again") && message.contains("expected fork-2")));
    assert!(list(&db).unwrap().is_empty());
}

#[tokio::test]
async fn mirror_drift_reselects_and_atomically_promotes_the_claimed_intake_once() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror-direct-drift.sqlite");
    upsert_thread(&db, "source", "project", "Source", 9, 10, 1.0).unwrap();
    mark_app_server_managed_target(&db, "source", 7).unwrap();
    mark_app_server_managed_target(&db, "new-target", 7).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("source")).unwrap();
    let backend = Arc::new(FakeBackend::default());
    let preprocessor = Arc::new(DirectMappingRacePreprocessor {
        db: db.clone(),
        changed: AtomicBool::new(false),
        seen: Mutex::new(Vec::new()),
    });
    let executor = executor(&temp, db.clone(), bridge, Arc::clone(&backend))
        .with_prompt_preprocessor(preprocessor.clone());

    executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "follow the mirror".into(),
            },
            ActionContext {
                channel_id: 10,
                user_id: 20,
                discord_message_id: Some(41),
                auto_queue_when_busy: true,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        preprocessor.seen.lock().unwrap().as_slice(),
        &["source", "new-target"]
    );
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "new-target");
    assert_eq!(jobs[0].prompt, "prepared:new-target:follow the mirror");
    assert_eq!(backend.starts.lock().await.len(), 1);
    assert!(list_prompt_intakes(&db).unwrap().is_empty());
}

struct MappingRacePreprocessor {
    db: PathBuf,
    transitions: Mutex<VecDeque<(String, String, String)>>,
    seen: Mutex<Vec<String>>,
}

impl MappingRacePreprocessor {
    fn new<'a>(
        db: PathBuf,
        transitions: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    ) -> Self {
        Self {
            db,
            transitions: Mutex::new(
                transitions
                    .into_iter()
                    .map(|(handoff, source, target)| (handoff.into(), source.into(), target.into()))
                    .collect(),
            ),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl PromptPreprocessor for MappingRacePreprocessor {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(thread_id.into());
            if let Some((handoff, source, target)) = self.transitions.lock().unwrap().pop_front() {
                begin_app_server_fork_handoff(
                    &self.db,
                    NewAppServerForkHandoff {
                        handoff_id: &handoff,
                        ambiguous_job_id: None,
                        source_thread_id: &source,
                        expected_generation: 7,
                        quarantine_reason: "test mapping race",
                    },
                )
                .unwrap();
                complete_app_server_fork_handoff(&self.db, &handoff, &target, 7).unwrap();
            }
            Ok(format!("prepared:{thread_id}:{prompt}"))
        })
    }
}

struct DirectMappingRacePreprocessor {
    db: PathBuf,
    changed: AtomicBool,
    seen: Mutex<Vec<String>>,
}

impl PromptPreprocessor for DirectMappingRacePreprocessor {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(thread_id.into());
            if !self.changed.swap(true, Ordering::SeqCst) {
                Connection::open(&self.db)
                    .unwrap()
                    .execute(
                        "DELETE FROM mirror_threads WHERE codex_thread_id = 'source'",
                        [],
                    )
                    .unwrap();
                upsert_thread(&self.db, "new-target", "project", "New", 9, 10, 2.0).unwrap();
            }
            Ok(format!("prepared:{thread_id}:{prompt}"))
        })
    }
}
