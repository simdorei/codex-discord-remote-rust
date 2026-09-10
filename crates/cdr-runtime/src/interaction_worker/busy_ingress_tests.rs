use super::*;
use crate::{
    discord_dispatch::{
        BoxDiscordFuture, DispatchOutcome, InteractionDispatcher, InteractionTransport,
    },
    test_support::{app_fixture, http_gate, message_fixture::MessageFixture},
};
use cdr_discord::{
    gateway::ingress::InteractionIngressTag, interaction_access::InteractionAccessPolicy,
};
use cdr_store::claims::{NewBusyChoice, create_busy_choice};
use serde_json::json;
use std::time::Duration;
use twilight_model::{
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

mod cleanup_overlap;
mod pro_contract;

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

fn seed(fixture: &MessageFixture) -> String {
    let db = fixture.executor.mirror_db();
    let generation = i64::try_from(fixture.server.generation()).unwrap();
    cdr_store::queue::enqueue(
        db,
        cdr_store::queue::NewQueueJob {
            job_id: "preparing",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "original",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(db, "preparing", &[], generation).unwrap();
    let choice = create_busy_choice(
        db,
        NewBusyChoice {
            owner_user_id: 3,
            channel_id: 42,
            target_thread_id: Some("thread-b"),
            prompt: "extra direction",
            allow_steer: false,
            now: crate::component_worker::now().unwrap(),
            time_to_live: 1800.0,
        },
    )
    .unwrap();
    cdr_store::control_binding::bind(db, &choice, "thread-b", None, Some("preparing")).unwrap();
    choice
}

async fn click(dispatcher: &InteractionDispatcher<Ack>, choice: &str, id: u64) -> DispatchOutcome {
    let interaction = serde_json::from_value(json!({
        "application_id":"2", "authorizing_integration_owners":{}, "channel_id":"42",
        "data":{"component_type":2,"custom_id":format!("codex_busy:{choice}:steer")},
        "entitlements":[], "id":id.to_string(), "locale":"en-US", "token":"fixture", "type":3,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},
        "version":1
    }))
    .unwrap();
    dispatcher
        .dispatch(
            &interaction,
            tokio::time::Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap()
}

async fn make_active(fixture: &MessageFixture) {
    cdr_store::queue::mark_running(
        fixture.executor.mirror_db(),
        "preparing",
        "original-turn",
        i64::try_from(fixture.server.generation()).unwrap(),
    )
    .unwrap();
    fixture
        .server
        .execute(
            cdr_app_server::requests::AppRequest {
                method: "test/active-turn",
                params: json!({"threadId":"thread-b","turnId":"original-turn"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn preparing_click_ingress_retries_only_proven_no_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let gate = http_gate::start().await;
    let http = Arc::new(Client::builder().proxy(gate.address.clone(), true).build());
    gate.release.send(()).unwrap();
    let fixture = MessageFixture::new(&temp, http.clone()).await;
    let choice = seed(&fixture);
    let (sender, mut receiver) = mpsc::channel(2);
    let dispatcher = InteractionDispatcher::new(
        Arc::new(Ack),
        InteractionAccessPolicy {
            allow_all_channels: true,
            ..Default::default()
        },
        false,
        sender,
        fixture.executor.mirror_db(),
    );
    assert_eq!(
        click(&dispatcher, &choice, 601).await,
        DispatchOutcome::Queued,
        "Steer (check) must reach original-turn validation, not be rejected by a stale display flag"
    );
    let work = receiver.recv().await.unwrap();
    reject_preparing(&fixture, &work, http.clone()).await;
    let db = fixture.executor.mirror_db();
    make_active(&fixture).await;
    assert_eq!(
        click(&dispatcher, &choice, 602).await,
        DispatchOutcome::Queued,
        "a new click after a proven no-send result is an explicit retry, not an unknown canonical duplicate"
    );
    let retry = receiver.recv().await.unwrap();
    assert_eq!(
        retry.processing_mode,
        crate::discord_dispatch::InteractionProcessingMode::Execute
    );
    for duplicate_id in [601, 603] {
        if click(&dispatcher, &choice, duplicate_id).await == DispatchOutcome::Queued {
            assert_ne!(
                receiver.recv().await.unwrap().processing_mode,
                crate::discord_dispatch::InteractionProcessingMode::Execute,
                "old events and extra clicks cannot compete with the admitted retry"
            );
        }
    }
    let mut custody =
        ExecutionCustody::begin(db, db, &retry.custody_ingress_id, retry.processing_mode).unwrap();
    process_interaction_work(
        &retry,
        &fixture.executor,
        &fixture.server,
        http,
        &mut custody,
    )
    .await
    .unwrap();
    custody
        .finish_success(&json!({"action_completed":true}))
        .unwrap();
    gate.entered.await.unwrap();
    gate.stop.send(()).unwrap();
    assert_eq!(gate.task.await.unwrap().len(), 1);
    fixture.server.close().await.unwrap();
    let calls = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    let steers: Vec<_> = calls
        .iter()
        .filter(|v| v["method"] == "turn/steer")
        .collect();
    assert_eq!(steers.len(), 1);
    assert_eq!(steers[0]["params"]["expectedTurnId"], "original-turn");
    assert!(!calls.iter().any(|v| matches!(
        v["method"].as_str(),
        Some("thread/fork" | "thread/resume" | "turn/start")
    )));
}

async fn reject_preparing(
    fixture: &MessageFixture,
    work: &InboundInteractionWork,
    http: Arc<Client>,
) {
    let db = fixture.executor.mirror_db();
    let mut custody =
        ExecutionCustody::begin(db, db, &work.custody_ingress_id, work.processing_mode).unwrap();
    let lock = fixture.executor.control_lock("thread-b").await.unwrap();
    let process =
        process_interaction_work(work, &fixture.executor, &fixture.server, http, &mut custody);
    let overlap = cleanup_overlap::create_while_claimed(fixture, lock);
    let (result, ()) = tokio::join!(process, overlap);
    let error = result.unwrap_err();
    custody.hold_failed().unwrap();
    let row = cdr_store::ingress::get(db, &work.custody_ingress_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        row.outcome.as_ref().map(|v| &v["control_dispatched"]),
        Some(&json!(false)),
        "the real worker must persist proof before allowing a new click: {error}"
    );
}
