use super::*;
use crate::discord_dispatch::{BoxDiscordFuture, InteractionTransport};
use crate::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    queue_runner::QueueCoordinator,
};
use twilight_model::{
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};
#[path = "../../../tests/support/usage_app_server.rs"]
mod app;
use crate::test_support::approval_http as http;

struct Ack;
impl InteractionTransport for Ack {
    fn acknowledge<'a>(
        &'a self,
        _: Id<InteractionMarker>,
        _: &'a str,
        _: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn update<'a>(&'a self, _: &'a str, _: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct ActualDispatcher {
    dispatcher: Arc<InteractionDispatcher<Ack>>,
    observed: mpsc::UnboundedSender<u64>,
}
impl InteractionHandler for ActualDispatcher {
    fn handle(&self, item: InteractionIngress) -> InteractionFuture {
        let handler = self.clone();
        Box::pin(async move {
            handler
                .dispatcher
                .dispatch(&item.event, item.received_at, item.tag)
                .await?;
            handler.observed.send(item.sequence).unwrap();
            Ok(())
        })
    }
}

fn event(id: u64, kind: &str) -> InteractionIngress {
    let options = match kind {
        "blank" => serde_json::json!([{"name":"model","type":3,"value":" "}]),
        "missing" => {
            serde_json::json!([{"name":"model","type":3,"value":"model-b"},{"name":"ref","type":3,"value":"missing-target"}])
        }
        _ => serde_json::json!([]),
    };
    let name = if kind == "valid" { "usage" } else { "settings" };
    InteractionIngress { sequence: id, received_at: Instant::now(), tag: InteractionIngressTag::Normal,
        event: Box::new(serde_json::from_value(serde_json::json!({
            "application_id":"2","authorizing_integration_owners":{},"channel_id":"42",
            "data":{"id":"1","name":name,"type":1,"options":options},
            "entitlements":[],"id":id.to_string(),"locale":"en-US","token":"fixture","type":2,
            "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},"version":1
        })).unwrap()) }
}

#[tokio::test]
async fn settings_rejections_cross_actual_lane_worker_and_next_valid_interaction() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("../../../tests/fixtures/action_state.sql"))
        .unwrap();
    let db = root.path().join("mirror.sqlite");
    cdr_store::schema::open_initialized(&db).unwrap();
    let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
    let server = Arc::new(app::start(root.path(), &root.path().join("rpc.jsonl"), "normal").await);
    let executor = Arc::new(
        ActionExecutor::new(
            state.clone(),
            db.clone(),
            bridge.clone(),
            Arc::new(QueueCoordinator::new(
                db.clone(),
                Arc::new(AppServerTurnBackend::new(server.clone())),
            )),
        )
        .with_server(server.clone()),
    );
    let fixture = http::start().await;
    let client = Arc::new(
        Client::builder()
            .proxy(fixture.address, true)
            .ratelimiter(None)
            .build(),
    );
    let (work_send, work_recv) = mpsc::channel(4);
    let dispatcher = Arc::new(
        InteractionDispatcher::new(
            Arc::new(Ack),
            InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            false,
            work_send,
            &db,
        )
        .with_settings_resolver(crate::settings_binding::SettingsTargetResolver::new(
            state,
            db.clone(),
            bridge.clone(),
        )),
    );
    let (input, received) = mpsc::channel(4);
    let (observed, mut events) = mpsc::unbounded_channel();
    let (shutdown, shutdown_rx) = watch::channel(false);
    let mut lane = tokio::spawn(run_lane(
        received,
        InteractionLane::Normal,
        1,
        ActualDispatcher {
            dispatcher: dispatcher.clone(),
            observed,
        },
        shutdown_rx,
    ));
    for (id, kind) in [(301, "blank"), (302, "missing"), (303, "valid")] {
        input.send(event(id, kind)).await.unwrap();
        tokio::select! {
            biased;
            ended = &mut lane => panic!("settings rejection stopped ingress: {ended:?}"),
            next = events.recv() => assert_eq!(next, Some(id)),
            () = tokio::time::sleep(Duration::from_secs(2)) => panic!("next interaction stalled"),
        }
    }
    assert_replay(&db, executor.settings_resolver()).await;
    shutdown.send(true).unwrap();
    drop(input);
    lane.await.unwrap().unwrap();
    drop(dispatcher);
    tokio::time::timeout(
        Duration::from_secs(5),
        crate::interaction_worker::run_interaction_worker(
            work_recv,
            executor,
            server.clone(),
            client,
        ),
    )
    .await
    .unwrap();
    fixture.stop.send(()).unwrap();
    let traffic = fixture.task.await.unwrap();
    assert_traffic(&traffic);
    for id in [301, 302] {
        let row = cdr_store::ingress::get(&db, &format!("interaction:{id}"))
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "completed");
        assert!(row.payload["request_rejection"].is_string());
        assert!(row.target_thread_id.is_none());
    }
    assert!(!bridge.path().exists());
    server.close().await.unwrap();
    let calls = std::fs::read_to_string(root.path().join("rpc.jsonl")).unwrap();
    assert!(!calls.contains("thread/") && !calls.contains("turn/"));
}

async fn assert_replay(
    db: &std::path::Path,
    resolver: crate::settings_binding::SettingsTargetResolver,
) {
    // A new in-memory claim cache still respects durable duplicate custody.
    let (send, mut recv) = mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::new(Ack),
        InteractionAccessPolicy {
            allow_all_channels: true,
            ..Default::default()
        },
        false,
        send,
        db,
    )
    .with_settings_resolver(resolver);
    let duplicate = event(302, "missing");
    assert_eq!(
        dispatcher
            .dispatch(&duplicate.event, duplicate.received_at, duplicate.tag)
            .await
            .unwrap(),
        crate::discord_dispatch::DispatchOutcome::Duplicate
    );
    assert!(recv.try_recv().is_err());
}

fn assert_traffic(traffic: &[(bool, serde_json::Value)]) {
    assert_eq!(traffic.len(), 3);
    assert!(traffic.iter().all(|v| !v.0));
    assert!(
        traffic[0].1["content"]
            .as_str()
            .unwrap()
            .starts_with("ERROR:")
    );
    assert!(
        traffic[1].1["content"]
            .as_str()
            .unwrap()
            .contains("missing-target")
    );
    assert!(
        traffic[2].1["content"]
            .as_str()
            .unwrap()
            .contains("total_tokens: 1234")
    );
}
