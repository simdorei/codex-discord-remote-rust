use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use cdr_discord::gateway::ingress::InteractionIngressTag;
use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_runtime::discord_dispatch::{
    BoxDiscordFuture, DiscordDispatchError, DispatchOutcome, InteractionClaimCache,
    InteractionDispatcher, InteractionTransport,
};
use cdr_runtime::restart_readiness::drain::{AdmissionGate, DrainFenceKey};
use serde_json::json;
use tokio::{sync::mpsc, time::Instant};
use twilight_model::{
    application::interaction::Interaction,
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

#[path = "support/interaction_dispatch.rs"]
mod dispatch_support;
use dispatch_support::DispatchDatabase;

#[derive(Default)]
struct FakeTransport {
    operations: Mutex<Vec<String>>,
    responses: Mutex<Vec<InteractionResponse>>,
    fail_ack: Mutex<bool>,
}

impl InteractionTransport for FakeTransport {
    fn acknowledge<'a>(
        &'a self,
        _id: Id<InteractionMarker>,
        _token: &'a str,
        response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            self.operations.lock().unwrap().push("ack".into());
            self.responses.lock().unwrap().push(response.clone());
            if *self.fail_ack.lock().unwrap() {
                Err("Discord unavailable".into())
            } else {
                Ok(())
            }
        })
    }

    fn update<'a>(&'a self, _token: &'a str, content: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            self.operations
                .lock()
                .unwrap()
                .push(format!("update:{content}"));
            Ok(())
        })
    }
}

fn interaction(id: u64, channel: u64, command: &str) -> Interaction {
    serde_json::from_value(json!({
        "application_id":"2",
        "authorizing_integration_owners":{},
        "channel_id":channel.to_string(),
        "data":{"id":"3","name":command,"type":1},
        "entitlements":[],
        "id":id.to_string(),
        "locale":"en-US",
        "token":"token",
        "type":2,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"20","username":"tester"},
        "version":1
    }))
    .unwrap()
}

fn policy() -> InteractionAccessPolicy {
    InteractionAccessPolicy {
        allowed_channel_ids: BTreeSet::from([10]),
        allowed_user_ids: BTreeSet::from([20]),
        mirrored_channel_ids: BTreeSet::new(),
        allow_all_channels: false,
    }
}

#[tokio::test]
async fn interaction_is_acknowledged_before_work_is_exposed_to_the_worker() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(FakeTransport::default());
    let (sender, mut receiver) = mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    );

    let outcome = dispatcher
        .dispatch(
            &interaction(401, 10, "help"),
            Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap();
    assert_eq!(outcome, DispatchOutcome::Queued);
    assert_eq!(*transport.operations.lock().unwrap(), vec!["ack"]);

    let work = receiver.recv().await.unwrap();
    assert_eq!(work.interaction_id.get(), 401);
    assert_eq!(work.channel_id.get(), 10);
    assert_eq!(work.user_id.get(), 20);
    assert_eq!(work.source_message_id, None);
    assert_eq!(work.interaction_token, "token");
}

#[tokio::test]
async fn queued_interaction_holds_restart_drain_until_worker_finishes() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(FakeTransport::default());
    let gate = AdmissionGate::new();
    let (sender, mut receiver) = mpsc::channel(1);
    let dispatcher =
        InteractionDispatcher::new(transport, policy(), false, sender, database.path())
            .with_admission_gate(gate.clone());

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(490, 10, "help"),
                Instant::now(),
                InteractionIngressTag::Normal,
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    let fence = DrainFenceKey::new("runtime-a", "42|638900000000000000", "nonce-a").unwrap();
    gate.seal(&fence).unwrap();
    assert!(!gate.is_drained_for(&fence), "queued work owns the permit");
    drop(receiver.recv().await.unwrap());
    assert!(gate.is_drained_for(&fence));
}

#[tokio::test]
async fn sealed_interaction_gets_retry_ack_and_never_enters_worker_queue() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(FakeTransport::default());
    let gate = AdmissionGate::new();
    let fence = DrainFenceKey::new("runtime-a", "42|638900000000000000", "nonce-a").unwrap();
    gate.seal(&fence).unwrap();
    let (sender, mut receiver) = mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    )
    .with_admission_gate(gate);

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(491, 10, "help"),
                Instant::now(),
                InteractionIngressTag::Normal,
            )
            .await
            .unwrap(),
        DispatchOutcome::Stopping
    );
    assert_eq!(*transport.operations.lock().unwrap(), vec!["ack"]);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn denied_interaction_is_acknowledged_but_never_queued() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(FakeTransport::default());
    let (sender, mut receiver) = mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    );

    let outcome = dispatcher
        .dispatch(
            &interaction(402, 99, "help"),
            Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap();

    assert_eq!(outcome, DispatchOutcome::RespondedWithoutWork);
    assert_eq!(*transport.operations.lock().unwrap(), vec!["ack"]);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn full_or_closed_queue_is_visible_in_the_deferred_response() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(FakeTransport::default());
    let (sender, receiver) = mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    );
    dispatcher
        .dispatch(
            &interaction(403, 10, "help"),
            Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap();
    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(404, 10, "help"),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::QueueFull
    );
    let full_content = transport.responses.lock().unwrap()[1]
        .data
        .as_ref()
        .and_then(|data| data.content.as_deref())
        .unwrap()
        .to_ascii_lowercase();
    assert!(full_content.contains("full"));
    let full = cdr_store::ingress::by_origin(database.path(), 404)
        .unwrap()
        .unwrap();
    assert_eq!(full.state, "held");
    assert_eq!(full.hold_reason, "interaction_queue_full");

    drop(receiver);
    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(405, 10, "help"),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Stopping
    );
    let stopping_content = transport.responses.lock().unwrap()[2]
        .data
        .as_ref()
        .and_then(|data| data.content.as_deref())
        .unwrap()
        .to_ascii_lowercase();
    assert!(stopping_content.contains("stopping"));
    let closed = cdr_store::ingress::by_origin(database.path(), 405)
        .unwrap()
        .unwrap();
    assert_eq!(closed.state, "held");
    assert_eq!(closed.hold_reason, "interaction_queue_closed");
}

#[tokio::test]
async fn failed_ack_is_not_misreported_as_queued() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(FakeTransport::default());
    *transport.fail_ack.lock().unwrap() = true;
    let (sender, mut receiver) = mpsc::channel(1);
    let dispatcher =
        InteractionDispatcher::new(transport, policy(), false, sender, database.path());

    let error = dispatcher
        .dispatch(
            &interaction(406, 10, "help"),
            Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap_err();

    assert_eq!(
        error,
        DiscordDispatchError::Acknowledge("Discord unavailable".into())
    );
    assert!(receiver.try_recv().is_err());
    let held = cdr_store::ingress::by_origin(database.path(), 406)
        .unwrap()
        .unwrap();
    assert_eq!(held.state, "held");
    assert_eq!(held.hold_reason, "discord_ack_failed");
}

#[tokio::test]
async fn failed_ack_is_held_and_never_authorizes_a_later_retry() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(FakeTransport::default());
    *transport.fail_ack.lock().unwrap() = true;
    let (sender, mut receiver) = mpsc::channel(1);
    let first = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender.clone(),
        database.path(),
    )
    .with_claim_cache(InteractionClaimCache::new(8));
    assert!(
        first
            .dispatch(
                &interaction(408, 10, "help"),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .is_err()
    );

    *transport.fail_ack.lock().unwrap() = false;
    let retry = InteractionDispatcher::new(transport, policy(), false, sender, database.path())
        .with_claim_cache(InteractionClaimCache::new(8));
    assert_eq!(
        retry
            .dispatch(
                &interaction(408, 10, "help"),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Duplicate
    );
    assert!(receiver.try_recv().is_err());
    let held = cdr_store::ingress::by_origin(database.path(), 408)
        .unwrap()
        .unwrap();
    assert_eq!(held.state, "held");
    assert_eq!(held.hold_reason, "discord_ack_failed");
}

#[tokio::test]
async fn duplicate_interaction_survives_claim_cache_recreation() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(FakeTransport::default());
    let claims = InteractionClaimCache::new(8);
    let (sender, mut receiver) = mpsc::channel(1);
    let first_dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender.clone(),
        database.path(),
    )
    .with_claim_cache(claims.clone());

    assert_eq!(
        first_dispatcher
            .dispatch(
                &interaction(407, 10, "help"),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    assert_eq!(receiver.recv().await.unwrap().channel_id.get(), 10);

    let recreated_dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    );
    assert_eq!(
        recreated_dispatcher
            .dispatch(
                &interaction(407, 10, "help"),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Duplicate
    );
    assert_eq!(*transport.operations.lock().unwrap(), vec!["ack"]);
    assert!(receiver.try_recv().is_err());
}
