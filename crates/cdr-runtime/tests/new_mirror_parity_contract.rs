use cdr_runtime::{
    action_executor::{ActionContext, ActionExecutor},
    app_backend::AppServerTurnBackend,
    bridge_state::BridgeState,
    command_plan::CommandAction,
    mirror_sync::{
        MirrorChannel, MirrorFuture, MirrorInventoryThread, MirrorSyncError, MirrorTransport,
    },
    queue_runner::QueueCoordinator,
};
use cdr_store::mapping::{mirrored_thread_id, upsert_project};
use std::sync::{Arc, Mutex};
use twilight_model::channel::ChannelType;
#[path = "support/new_ingress_shapes.rs"]
mod ingress_shapes;
#[path = "support/action_app_server.rs"]
mod support;

const CONTEXT: ActionContext = ActionContext {
    channel_id: 99,
    user_id: 20,
    discord_message_id: Some(30),
    auto_queue_when_busy: true,
};

#[path = "support/new_mirror_remote.rs"]
mod remote;
use remote::Remote;
#[path = "support/new_context_replay.rs"]
mod context_replay;
#[path = "support/new_persistence_assertions.rs"]
mod persistence;
#[path = "support/new_project_race.rs"]
mod project_race;

async fn scenario(
    fail: bool,
    ingress_kind: Option<cdr_store::ingress::IngressKind>,
    concurrent: bool,
) {
    scenario_with_persistence(fail, ingress_kind, concurrent, true, 99).await;
}

#[allow(
    clippy::too_many_lines,
    reason = "one temporal creation/persistence/replay contract with shared setup and cleanup"
)]
async fn scenario_with_persistence(
    fail: bool,
    ingress_kind: Option<cdr_store::ingress::IngressKind>,
    concurrent: bool,
    persist: bool,
    origin: u64,
) {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let state = temp.path().join("state.sqlite");
    let server = Arc::new(if persist {
        support::start_persisting_server(&temp, &log, &state).await
    } else {
        support::start_fake_server(&temp, &log).await
    });
    let db = temp.path().join("mirror.sqlite");
    let cwd = temp.path().to_string_lossy().into_owned();
    persistence::setup_project(&state, &db, &cwd, origin);
    let queue = Arc::new(QueueCoordinator::new(
        db.clone(),
        Arc::new(AppServerTurnBackend::new(server.clone())),
    ));
    let executor = ActionExecutor::new(
        state.clone(),
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        queue.clone(),
    )
    .with_server(server.clone());
    let remote = Arc::new(Remote {
        fail,
        pause_create: concurrent.then(|| {
            (
                Arc::new(tokio::sync::Notify::new()),
                Arc::new(tokio::sync::Notify::new()),
            )
        }),
        ..Remote::default()
    });
    executor
        .set_mirror_transport(remote.clone(), Some(1))
        .unwrap();
    let context = ActionContext {
        channel_id: origin,
        ..CONTEXT
    };
    let action = CommandAction::New {
        prompt: "새 작업".into(),
    };
    let original = ingress_kind.map(|kind| ingress_shapes::admit(&db, kind, &action, origin));
    let started = std::time::Instant::now();
    let result = remote.execute_new(&executor, &action, context).await;
    if !persist {
        assert!(
            result.is_ok(),
            "accepted execution must return its normal first reply without waiting for rollout persistence: {result:?}"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(
            result.unwrap().text,
            "In progress\nmessage: 새 작업\n새 대화: <#100>"
        );
        let original = cdr_store::ingress::by_origin(&db, 30).unwrap().unwrap();
        assert_eq!(original.target_thread_id.as_deref(), Some("new-thread"));
        assert!(
            original.owner_id.is_some(),
            "accepted execution remains owned, never recreated"
        );
        persistence::verify_rpc_counts(&log, &cwd, false);
        server.close().await.unwrap();
        return;
    }
    if fail {
        assert!(result.unwrap_err().to_string().contains("HTTP 403"));
        assert!(cdr_store::queue::list(&db).unwrap().is_empty());
        assert!(
            executor
                .execute_with_context(action, context)
                .await
                .is_err()
        );
    } else {
        let result = result.unwrap();
        assert!(result.text.contains("<#100>"));
        assert_eq!(
            mirrored_thread_id(&db, Some(100)).unwrap().as_deref(),
            Some("new-thread")
        );
        let jobs = cdr_store::queue::list(&db).unwrap();
        assert_eq!((jobs.len(), jobs[0].channel_id), (1, 100));
        persistence::verify_persisted(&state, &cwd, &server).await;
        executor
            .execute_with_context(action, context)
            .await
            .unwrap();
        if let Some((key, payload)) = &original {
            let saved = cdr_store::ingress::get(&db, key).unwrap().unwrap();
            assert_eq!(
                &saved.payload, payload,
                "the original gateway journal must not be rewritten"
            );
        }
        persistence::verify_completion(&db, &queue).await;
    }
    assert_eq!(*remote.creates.lock().unwrap(), 1);
    persistence::verify_rpc_counts(&log, &cwd, fail);
    server.close().await.unwrap();
}
#[tokio::test]
async fn new_uses_project_and_links_new_room_once() {
    scenario(false, None, false).await;
}
#[tokio::test]
async fn room_failure_keeps_prompt_without_recreating_or_sending_elsewhere() {
    scenario(true, None, false).await;
}

#[tokio::test]
async fn actual_prefix_journal_hands_off_the_first_prompt_without_rewriting_payload() {
    scenario(false, Some(cdr_store::ingress::IngressKind::Message), false).await;
}

#[tokio::test]
async fn actual_slash_journal_hands_off_the_first_prompt_without_rewriting_payload() {
    scenario(
        false,
        Some(cdr_store::ingress::IngressKind::Interaction),
        false,
    )
    .await;
}

#[tokio::test]
async fn duplicate_during_room_create_cannot_hold_the_original_new_request() {
    scenario(false, Some(cdr_store::ingress::IngressKind::Message), true).await;
}

#[tokio::test]
async fn new_acceptance_replies_before_persistence_without_recreating_execution() {
    scenario_with_persistence(
        false,
        Some(cdr_store::ingress::IngressKind::Message),
        false,
        false,
        99,
    )
    .await;
}

#[tokio::test]
async fn new_from_existing_thread_keeps_its_project_and_creates_a_sibling_room() {
    scenario_with_persistence(
        false,
        Some(cdr_store::ingress::IngressKind::Message),
        false,
        true,
        101,
    )
    .await;
}
