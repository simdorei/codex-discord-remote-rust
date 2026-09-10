use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cdr_runtime::action_executor::{ActionContext, ActionError};
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use cdr_store::queue::{
    NewAppServerForkHandoff, begin_app_server_fork_handoff, complete_app_server_fork_handoff,
    completed_app_server_fork_target_for_source, list, mark_app_server_managed_target,
};
use tokio::sync::Notify;

#[path = "support/action_target.rs"]
mod action_target;
use action_target::{FakeBackend, executor};

#[tokio::test]
async fn concurrent_action_reprepares_after_another_action_moves_its_selected_target() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "selected", 7).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("selected")).unwrap();
    let backend = Arc::new(FakeBackend {
        fork_targets: BTreeMap::from([("selected".into(), "moved".into())]),
        resume_conflicts: BTreeSet::from(["selected".into()]),
        ..FakeBackend::default()
    });
    let preprocessor = Arc::new(ConcurrentMovePreprocessor {
        db: db.clone(),
        second_prepared: Notify::new(),
        seen: Mutex::new(Vec::new()),
    });
    let executor = executor(&temp, db.clone(), bridge, Arc::clone(&backend))
        .with_prompt_preprocessor(preprocessor.clone());

    let actions = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        tokio::join!(
            executor.execute_with_context(
                CommandAction::Ask {
                    prompt: "first".into(),
                },
                action_context(301),
            ),
            executor.execute_with_context(
                CommandAction::Ask {
                    prompt: "second".into(),
                },
                action_context(302),
            ),
        )
    })
    .await
    .expect("concurrent actions must finish");
    actions.0.unwrap();
    actions.1.unwrap();

    assert_eq!(*backend.forks.lock().await, vec!["selected"]);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|job| job.target_thread_id == "moved"));
    let second = jobs
        .iter()
        .find(|job| job.discord_message_id == Some(302))
        .unwrap();
    assert_eq!(second.prompt, "prepared:moved:second");
    assert_eq!(
        preprocessor.seen.lock().unwrap().as_slice(),
        [
            ("first".into(), "selected".into()),
            ("second".into(), "selected".into()),
            ("second".into(), "moved".into()),
        ]
    );
}

#[tokio::test]
async fn a_second_selected_target_move_fails_closed_without_an_enqueue() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, "selected", 7).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("selected")).unwrap();
    let preprocessor = Arc::new(MoveEveryPrepare {
        db: db.clone(),
        transitions: Mutex::new(VecDeque::from([
            ("move-1".into(), "selected".into(), "moved-a".into()),
            ("move-2".into(), "moved-a".into(), "moved-b".into()),
        ])),
    });
    let executor = executor(&temp, db.clone(), bridge, Arc::new(FakeBackend::default()))
        .with_prompt_preprocessor(preprocessor);

    let error = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "never enqueue stale".into(),
            },
            action_context(303),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, ActionError::Invalid(message) if
        message.contains("target changed again")
            && message.contains("moved source thread moved-a to moved-b")
            && message.contains("no request was queued")));
    assert!(list(&db).unwrap().is_empty());
}

fn action_context(message_id: u64) -> ActionContext {
    ActionContext {
        channel_id: 99,
        user_id: 20,
        discord_message_id: Some(message_id),
        auto_queue_when_busy: true,
    }
}

struct ConcurrentMovePreprocessor {
    db: PathBuf,
    second_prepared: Notify,
    seen: Mutex<Vec<(String, String)>>,
}

impl PromptPreprocessor for ConcurrentMovePreprocessor {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            self.seen
                .lock()
                .unwrap()
                .push((prompt.into(), thread_id.into()));
            if thread_id == "selected" {
                if prompt == "first" {
                    self.second_prepared.notified().await;
                } else if prompt == "second" {
                    self.second_prepared.notify_one();
                    while completed_app_server_fork_target_for_source(&self.db, "selected")
                        .unwrap()
                        .as_deref()
                        != Some("moved")
                    {
                        tokio::task::yield_now().await;
                    }
                }
            }
            Ok(format!("prepared:{thread_id}:{prompt}"))
        })
    }
}

struct MoveEveryPrepare {
    db: PathBuf,
    transitions: Mutex<VecDeque<(String, String, String)>>,
}

impl PromptPreprocessor for MoveEveryPrepare {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            if let Some((handoff, source, target)) = self.transitions.lock().unwrap().pop_front() {
                complete_move(&self.db, &handoff, &source, &target);
            }
            Ok(format!("prepared:{thread_id}:{prompt}"))
        })
    }
}

fn complete_move(db: &Path, handoff: &str, source: &str, target: &str) {
    begin_app_server_fork_handoff(
        db,
        NewAppServerForkHandoff {
            handoff_id: handoff,
            ambiguous_job_id: None,
            source_thread_id: source,
            expected_generation: 7,
            quarantine_reason: "selected race test",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(db, handoff, target, 7).unwrap();
}
