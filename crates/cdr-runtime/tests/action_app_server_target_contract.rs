use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use cdr_runtime::action_executor::ActionContext;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_store::mapping::{thread_channels, upsert_thread};
use cdr_store::queue::{enqueue, list};

#[path = "support/action_target.rs"]
mod action_target;
use action_target::{FakeBackend, executor, job, seed_handoff};

#[tokio::test]
async fn stale_selected_source_follows_completed_fork_and_updates_bridge_state() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_handoff(&db, "seed", "thread-a", "thread-a-bot", 9, 10);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    bridge
        .remember_thread_settings("thread-a", Some("model-a"), Some("max"), Some("fast"))
        .unwrap();
    let backend = Arc::new(FakeBackend::default());
    let executor = executor(&temp, db, Arc::clone(&bridge), Arc::clone(&backend));

    let result = executor
        .execute(
            CommandAction::Ask {
                prompt: "continue".into(),
            },
            99,
            20,
        )
        .await
        .unwrap();

    assert_eq!(result.text, "In progress\nmessage: continue");
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("thread-a-bot")
    );
    assert_eq!(
        bridge
            .thread_settings("thread-a-bot")
            .unwrap()
            .model
            .as_deref(),
        Some("model-a")
    );
    assert_eq!(*backend.forks.lock().await, Vec::<String>::new());
    assert_eq!(
        *backend.starts.lock().await,
        vec![("thread-a-bot".into(), "continue".into())]
    );
}

#[tokio::test]
async fn fresh_unmapped_selected_thread_is_forked_before_a_write() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let backend = Arc::new(FakeBackend {
        fork_targets: BTreeMap::from([("thread-a".into(), "selected-fork".into())]),
        ..FakeBackend::default()
    });
    let executor = executor(&temp, db.clone(), Arc::clone(&bridge), Arc::clone(&backend));

    let result = executor
        .execute(
            CommandAction::Ask {
                prompt: "continue".into(),
            },
            99,
            20,
        )
        .await
        .unwrap();

    assert_eq!(result.text, "In progress\nmessage: continue");
    assert_eq!(*backend.forks.lock().await, vec!["thread-a"]);
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("selected-fork")
    );
    assert_eq!(
        *backend.starts.lock().await,
        vec![("selected-fork".into(), "continue".into())]
    );
    assert_eq!(thread_channels(&db, "thread-a").unwrap(), None);
    assert_eq!(thread_channels(&db, "selected-fork").unwrap(), None);
}

#[tokio::test]
async fn retract_refuses_a_legacy_moved_selection_without_cancelling_its_copy() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_handoff(&db, "seed", "thread-a", "thread-a-bot", 9, 10);
    enqueue(&db, job("queued", "thread-a-bot", 99, "pending", 2.0)).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let backend = Arc::new(FakeBackend::default());
    let executor = executor(&temp, db.clone(), Arc::clone(&bridge), Arc::clone(&backend));

    let before = list(&db).unwrap();
    let error = executor
        .execute(CommandAction::Retract { reference: None }, 99, 20)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("thread-a-bot"));
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("thread-a")
    );
    assert_eq!(list(&db).unwrap(), before);
    assert!(backend.forks.lock().await.is_empty());
}

#[tokio::test]
async fn active_writer_button_recovery_never_waits_for_an_unrelated_broken_target() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "thread-a", "project", "Main", 9, 10, 1.0).unwrap();
    seed_handoff(&db, "broken-seed", "broken-source", "aaa-broken", 199, 200);
    enqueue(&db, job("broken-job", "aaa-broken", 200, "blocked", 1.0)).unwrap();
    let backend = Arc::new(FakeBackend {
        fork_targets: BTreeMap::from([
            ("thread-a".into(), "managed-main".into()),
            ("managed-main".into(), "managed-main-2".into()),
        ]),
        resume_conflicts: BTreeSet::from(["managed-main".into()]),
        resume_hangs: BTreeSet::from(["aaa-broken".into()]),
        ..FakeBackend::default()
    });
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let executor = executor(&temp, db.clone(), bridge, Arc::clone(&backend));

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        executor.execute_with_context(
            CommandAction::Ask {
                prompt: "do it".into(),
            },
            ActionContext {
                channel_id: 10,
                user_id: 20,
                discord_message_id: Some(30),
                auto_queue_when_busy: true,
            },
        ),
    )
    .await
    .expect("action must not await unrelated recovery")
    .unwrap();

    assert_eq!(result.text, "In progress\nmessage: do it");
    assert_eq!(
        *backend.forks.lock().await,
        vec!["thread-a", "managed-main"]
    );
    assert!(
        !backend
            .resumes
            .lock()
            .await
            .iter()
            .any(|id| id == "aaa-broken")
    );
    assert!(list(&db).unwrap().iter().any(|job| {
        job.target_thread_id == "managed-main-2" && job.turn_id.as_deref().is_some()
    }));
}
