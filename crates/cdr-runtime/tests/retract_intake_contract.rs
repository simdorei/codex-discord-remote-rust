use cdr_runtime::{
    action_executor::ActionExecutor,
    bridge_state::BridgeState,
    command_plan::CommandAction,
    queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord},
};
use cdr_store::{
    prompt_intake::{self, NewPromptIntake},
    queue::{self, NewQueueJob},
};
use std::sync::Arc;

struct NoRpc;
impl TurnBackend for NoRpc {
    fn generation(&self) -> u64 {
        1
    }
    fn active_turn_id<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { panic!("retract must not issue backend calls") })
    }
    fn read_turns<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { panic!("retract must not issue backend calls") })
    }
    fn resume_thread<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { panic!("retract must not resume") })
    }
    fn start_turn<'a>(&'a self, _: &'a str, _: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async { panic!("retract must not start") })
    }
}

fn fixture(temp: &tempfile::TempDir) -> ActionExecutor<NoRpc> {
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0).unwrap();
    ActionExecutor::new(
        state,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(db, Arc::new(NoRpc))),
    )
}

fn intake(id: &str, owner: i64, created: f64) -> NewPromptIntake<'_> {
    NewPromptIntake {
        job_id: id,
        target_thread_id: "thread-b",
        channel_id: 42,
        owner_user_id: Some(owner),
        discord_message_id: None,
        raw_prompt: "아직 실행 전",
        auto_queue_when_busy: true,
        require_current_mirror: true,
        created_at: created,
    }
}

fn queued(id: &str, created: f64) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id: id,
        target_thread_id: "thread-b",
        channel_id: 42,
        owner_user_id: Some(3),
        discord_message_id: None,
        app_server_generation: 1,
        prompt: "queued",
        queued: true,
        ack_sent: true,
        created_at: created,
    }
}

#[tokio::test]
async fn retract_includes_pre_queue_intake_and_preserves_other_sender() {
    let temp = tempfile::tempdir().unwrap();
    let executor = fixture(&temp);
    let db = executor.mirror_db();
    prompt_intake::admit_prompt_intake(db, intake("own", 3, 1.0)).unwrap();
    let foreign = prompt_intake::admit_prompt_intake(db, intake("foreign", 4, 2.0))
        .unwrap()
        .intake;
    let result = executor
        .execute(CommandAction::Retract { reference: None }, 42, 3)
        .await
        .unwrap();
    assert!(result.text.contains("own"), "{}", result.text);
    assert!(
        prompt_intake::get_prompt_intake(db, "own")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        prompt_intake::get_prompt_intake(db, "foreign").unwrap(),
        Some(foreign)
    );
    assert!(
        prompt_intake::admit_prompt_intake(db, intake("own", 3, 1.0)).is_err(),
        "cancellation must remain durable across stale intake retry"
    );
}

#[tokio::test]
async fn retract_chooses_latest_across_queue_and_intake() {
    let temp = tempfile::tempdir().unwrap();
    let executor = fixture(&temp);
    let db = executor.mirror_db();
    let older = queue::enqueue(db, queued("older", 1.0)).unwrap();
    prompt_intake::admit_prompt_intake(db, intake("newer", 3, 2.0)).unwrap();
    let result = executor
        .execute(CommandAction::Retract { reference: None }, 42, 3)
        .await
        .unwrap();
    assert!(result.text.contains("newer"), "{}", result.text);
    assert_eq!(queue::list(db).unwrap(), vec![older.job]);
    assert!(
        prompt_intake::get_prompt_intake(db, "newer")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn no_unstarted_request_returns_an_explicit_started_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let executor = fixture(&temp);
    let db = executor.mirror_db();
    queue::enqueue(db, queued("starting", 1.0)).unwrap();
    queue::begin_attempt(db, "starting", &[], 1).unwrap();
    let before = queue::list(db).unwrap();
    assert!(
        executor
            .execute(CommandAction::Retract { reference: None }, 42, 3)
            .await
            .is_err()
    );
    assert_eq!(queue::list(db).unwrap(), before);
}

#[tokio::test]
async fn explicit_original_reference_never_follows_a_legacy_copy_to_cancel_work() {
    let temp = tempfile::tempdir().unwrap();
    let executor = fixture(&temp);
    let db = executor.mirror_db();
    queue::enqueue(db, queued("original-pending", 1.0)).unwrap();
    queue::begin_app_server_fork_handoff(
        db,
        queue::NewAppServerForkHandoff {
            handoff_id: "legacy-move",
            ambiguous_job_id: None,
            source_thread_id: "thread-b",
            expected_generation: 1,
            quarantine_reason: "isolated legacy fixture",
        },
    )
    .unwrap();
    queue::complete_app_server_fork_handoff(db, "legacy-move", "thread-a", 1).unwrap();
    let before = queue::list(db).unwrap();
    assert_eq!(before[0].target_thread_id, "thread-a");
    let result = executor
        .execute(
            CommandAction::Retract {
                reference: Some("thread-b".into()),
            },
            42,
            3,
        )
        .await;
    assert!(
        result.is_err(),
        "an explicit original target must not cancel its old copy's request"
    );
    assert_eq!(queue::list(db).unwrap(), before);
}
