use std::collections::BTreeSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::task::Poll;

use cdr_discord::gateway::ingress::InteractionIngressTag;
use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_runtime::discord_dispatch::{
    BoxDiscordFuture, DispatchOutcome, InteractionClaimCache, InteractionDispatcher,
    InteractionTransport,
};
use serde_json::json;
use tokio::{
    sync::{Notify, mpsc},
    time::Instant,
};
use twilight_model::{
    application::interaction::Interaction,
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

#[path = "support/interaction_dispatch.rs"]
mod dispatch_support;
use dispatch_support::DispatchDatabase;

#[derive(Default)]
struct BlockingTransport {
    acknowledgements: Mutex<Vec<u64>>,
    block_first: AtomicBool,
    entered: Notify,
    release: Notify,
}

#[derive(Default)]
struct PendingFirstTransport {
    acknowledgement_attempts: AtomicUsize,
}

#[derive(Default)]
struct ImmediateTransport {
    acknowledgement_attempts: AtomicUsize,
}

impl InteractionTransport for PendingFirstTransport {
    fn acknowledge<'a>(
        &'a self,
        _id: Id<InteractionMarker>,
        _token: &'a str,
        _response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            let attempt = self.acknowledgement_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                std::future::pending::<()>().await;
            }
            Ok(())
        })
    }

    fn update<'a>(&'a self, _token: &'a str, _content: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

impl InteractionTransport for ImmediateTransport {
    fn acknowledge<'a>(
        &'a self,
        _id: Id<InteractionMarker>,
        _token: &'a str,
        _response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            self.acknowledgement_attempts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn update<'a>(&'a self, _token: &'a str, _content: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

impl InteractionTransport for BlockingTransport {
    fn acknowledge<'a>(
        &'a self,
        id: Id<InteractionMarker>,
        _token: &'a str,
        _response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            self.acknowledgements.lock().unwrap().push(id.get());
            if id.get() == 500 && !self.block_first.swap(true, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(())
        })
    }

    fn update<'a>(&'a self, _token: &'a str, _content: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn interaction(id: u64) -> Interaction {
    serde_json::from_value(json!({
        "application_id":"2",
        "authorizing_integration_owners":{},
        "channel_id":"10",
        "data":{"id":"3","name":"help","type":1},
        "entitlements":[],
        "id":id.to_string(),
        "locale":"en-US",
        "token":format!("token-{id}"),
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
async fn capacity_pressure_never_evicts_an_acknowledgement_in_flight() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(BlockingTransport::default());
    let claims = InteractionClaimCache::new(2);
    let (sender, mut receiver) = mpsc::channel(8);
    let dispatcher = Arc::new(
        InteractionDispatcher::new(
            Arc::clone(&transport),
            policy(),
            false,
            sender,
            database.path(),
        )
        .with_claim_cache(claims),
    );

    let first = tokio::spawn({
        let dispatcher = Arc::clone(&dispatcher);
        async move {
            dispatcher
                .dispatch(
                    &interaction(500),
                    Instant::now(),
                    InteractionIngressTag::Normal,
                )
                .await
        }
    });
    transport.entered.notified().await;
    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(501),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(502),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );

    let duplicate = dispatcher
        .dispatch(
            &interaction(500),
            Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap();
    assert_eq!(duplicate, DispatchOutcome::DuplicatePending);
    transport.release.notify_one();
    assert_eq!(first.await.unwrap().unwrap(), DispatchOutcome::Queued);

    let mut queued = Vec::new();
    while let Ok(work) = receiver.try_recv() {
        queued.push(work.interaction_id.get());
    }
    assert_eq!(queued.iter().filter(|id| **id == 500).count(), 1);
    assert_eq!(
        transport
            .acknowledgements
            .lock()
            .unwrap()
            .iter()
            .filter(|id| **id == 500)
            .count(),
        1
    );
}

#[tokio::test]
async fn cancelling_an_acknowledgement_holds_custody_and_suppresses_retry() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(PendingFirstTransport::default());
    let claims = InteractionClaimCache::new(1);
    let (sender, mut receiver) = mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    )
    .with_claim_cache(claims);
    let first_interaction = interaction(600);
    let mut first_dispatch = Box::pin(dispatcher.dispatch(
        &first_interaction,
        Instant::now(),
        InteractionIngressTag::Normal,
    ));

    assert!(matches!(
        futures_util::poll!(first_dispatch.as_mut()),
        Poll::Pending
    ));
    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(600),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::DuplicatePending
    );

    drop(first_dispatch);
    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(600),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Duplicate
    );
    assert_eq!(transport.acknowledgement_attempts.load(Ordering::SeqCst), 1);
    assert!(receiver.try_recv().is_err());
    let held = cdr_store::ingress::by_origin(database.path(), 600)
        .unwrap()
        .unwrap();
    assert_eq!(held.state, "held");
    assert_eq!(held.hold_reason, "interaction_dispatch_cancelled");
}

#[tokio::test]
async fn closed_queue_is_durably_rejected_before_acknowledgement() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(ImmediateTransport::default());
    let claims = InteractionClaimCache::new(1);
    let (sender, receiver) = mpsc::channel(1);
    drop(receiver);
    let dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    )
    .with_claim_cache(claims);
    let outcome = dispatcher
        .dispatch(
            &interaction(601),
            Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap();
    assert_eq!(outcome, DispatchOutcome::Stopping);
    assert_eq!(transport.acknowledgement_attempts.load(Ordering::SeqCst), 1);
    let held = cdr_store::ingress::by_origin(database.path(), 601)
        .unwrap()
        .unwrap();
    assert_eq!(held.state, "held");
    assert_eq!(held.hold_reason, "interaction_queue_closed");
    let duplicate = dispatcher
        .dispatch(
            &interaction(601),
            Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap();
    assert_eq!(duplicate, DispatchOutcome::Duplicate);
    assert_eq!(transport.acknowledgement_attempts.load(Ordering::SeqCst), 1);
}
