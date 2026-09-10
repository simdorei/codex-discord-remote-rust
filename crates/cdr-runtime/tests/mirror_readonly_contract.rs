use cdr_runtime::{
    action_executor::ActionExecutor,
    bridge_state::BridgeState,
    command_plan::CommandAction,
    message_plan::{IncomingMessage, MessagePlan, plan_message},
    mirror_sync::{
        MirrorChannel, MirrorFuture, MirrorInventoryThread, MirrorSyncError, MirrorTransport,
    },
    queue_runner::QueueCoordinator,
};
use cdr_store::mapping::{mirror_targets, upsert_thread};
use std::{collections::BTreeSet, sync::Arc};
use twilight_model::channel::ChannelType;
#[path = "support/mirror_readonly_projects.rs"]
mod project_cases;
#[path = "support/action_target.rs"]
#[allow(dead_code)]
mod support;

struct ReadOnlyRemote;
impl MirrorTransport for ReadOnlyRemote {
    fn channel(&self, id: u64) -> MirrorFuture<'_, Option<MirrorChannel>> {
        Box::pin(async move {
            if id == 404 {
                return Ok(None);
            }
            if id == 403 {
                return Err(MirrorSyncError::Discord("HTTP 403: Missing Access".into()));
            }
            Ok(Some(MirrorChannel {
                id,
                guild_id: Some(1),
                parent_id: (!(90..=91).contains(&id)).then_some(90),
                kind: if (90..=91).contains(&id) {
                    ChannelType::GuildText
                } else {
                    ChannelType::PublicThread
                },
                name: "unchanged".into(),
                topic: None,
                archived: false,
            }))
        })
    }
    fn channels(&self, _: u64) -> MirrorFuture<'_, Vec<MirrorChannel>> {
        panic!("inspection must not enter sync inventory")
    }
    fn thread_inventory(&self, _: u64, _: u64) -> MirrorFuture<'_, Vec<MirrorInventoryThread>> {
        panic!("no cleanup inventory")
    }
    fn create<'a>(
        &'a self,
        _: u64,
        _: Option<u64>,
        _: ChannelType,
        _: &'a str,
        _: Option<&'a str>,
    ) -> MirrorFuture<'a, MirrorChannel> {
        panic!("read-only command attempted create")
    }
    fn update<'a>(
        &'a self,
        _: &'a MirrorChannel,
        _: &'a str,
        _: Option<&'a str>,
        _: bool,
    ) -> MirrorFuture<'a, ()> {
        panic!("read-only command attempted update")
    }
    fn delete(&self, _: u64) -> MirrorFuture<'_, ()> {
        panic!("read-only command attempted delete")
    }
}

fn action(text: &str) -> CommandAction {
    let input = IncomingMessage {
        content: text,
        message_content_enabled: true,
        channel_allowed: true,
        user_allowed: true,
        author_is_bot: false,
        author_is_self: false,
        author_mentions_bridge: false,
        has_attachments: false,
        mirrored_target: true,
        mentioned_user_ids: BTreeSet::new(),
        required_plain_ask_user_ids: BTreeSet::new(),
    };
    let MessagePlan::Execute(action) = plan_message(&input).unwrap() else {
        panic!("command not routed")
    };
    action
}

fn slash_check() -> CommandAction {
    use cdr_discord::interaction::{RoutedWork, route_command};
    use twilight_model::application::{
        command::CommandType,
        interaction::{InteractionType, application_command::CommandData},
    };
    let data = CommandData {
        guild_id: None,
        id: twilight_model::id::Id::new(1),
        name: "mirror_check".into(),
        kind: CommandType::ChatInput,
        options: vec![],
        resolved: None,
        target_id: None,
    };
    let route = route_command(&data, InteractionType::ApplicationCommand, false).unwrap();
    let Some(RoutedWork::Slash(invocation)) = route.work else {
        panic!("slash route expected")
    };
    cdr_runtime::command_plan::plan_slash(&invocation).unwrap()
}

#[tokio::test]
async fn real_prefix_check_and_list_never_sync_or_change_mapping() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let state_connection = rusqlite::Connection::open(&state).unwrap();
    state_connection
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let rollout = tempfile::NamedTempFile::new_in(temp.path()).unwrap();
    state_connection
        .execute(
            "UPDATE threads SET rollout_path = ?1 WHERE archived = 0",
            [rollout.path().to_str().unwrap()],
        )
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "thread-a", "alpha", "A", 90, 100, 1.0).unwrap();
    upsert_thread(&db, "thread-b", "alpha", "B", 90, 101, 1.0).unwrap();
    cdr_store::mapping::upsert_project(&db, "alpha", "alpha", 90, 1.0, |a, b| a == b).unwrap();
    let before = mirror_targets(&db, i64::MAX).unwrap();
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(support::FakeBackend::default()),
        )),
    );
    executor
        .set_mirror_transport(Arc::new(ReadOnlyRemote), Some(1))
        .unwrap();
    let file_before = std::fs::read(&db).unwrap();
    for input in [
        "!mirror check",
        "!mirror check 1",
        "!mirror list",
        "!mirror list 1",
        "!mirror doctor",
    ] {
        let planned = action(input);
        assert!(
            !matches!(planned, CommandAction::BridgeSync { .. }),
            "{input} must be read-only"
        );
        let result = executor.execute(planned, 90, 20).await.unwrap();
        assert!(result.text.contains("read-only"), "{}", result.text);
        assert!(result.text.contains("status: ok"), "{}", result.text);
        assert!(result.text.contains("missing_mapping: 0"));
        assert!(result.text.contains("missing_rollouts: 0"));
        assert_eq!(std::fs::read(&db).unwrap(), file_before);
        assert_eq!(mirror_targets(&db, i64::MAX).unwrap(), before);
    }
    let slash = executor.execute(slash_check(), 90, 20).await.unwrap();
    let prefix = executor
        .execute(action("!mirror check"), 90, 20)
        .await
        .unwrap();
    assert_eq!(slash.text, prefix.text);
    assert_eq!(std::fs::read(&db).unwrap(), file_before);
    assert!(matches!(
        action("!mirror sync"),
        CommandAction::BridgeSync { limit: None }
    ));
}

#[tokio::test]
async fn check_distinguishes_missing_duplicate_stale_and_access_errors_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    for (id, room) in [("thread-a", 403), ("ghost", 403), ("thread-old", 404)] {
        upsert_thread(&db, id, "project", id, 90, room, 1.0).unwrap();
    }
    let before = mirror_targets(&db, i64::MAX).unwrap();
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(support::FakeBackend::default()),
        )),
    );
    executor
        .set_mirror_transport(Arc::new(ReadOnlyRemote), Some(1))
        .unwrap();
    let result = executor
        .execute(CommandAction::MirrorCheck, 90, 20)
        .await
        .unwrap();
    for expected in [
        "missing_mapping: 1",
        "duplicate_rooms: 1",
        "stale_mappings: 2",
        "HTTP 403",
        "missing_room",
        "thread-b",
    ] {
        assert!(
            result.text.contains(expected),
            "missing {expected}: {}",
            result.text
        );
    }
    let limited = executor
        .execute(action("!mirror check 1"), 90, 20)
        .await
        .unwrap();
    for summary in [
        "missing_mapping: 1",
        "duplicate_rooms: 1",
        "stale_mappings: 2",
    ] {
        assert!(limited.text.contains(summary));
    }
    assert!(limited.text.contains("details: 1/"));
    assert_eq!(mirror_targets(&db, i64::MAX).unwrap(), before);
}
