use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use cdr_discord::gateway::ingress::InteractionIngressTag;
use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_runtime::discord_dispatch::{
    BoxDiscordFuture, DispatchOutcome, INTERACTION_ACK_BUDGET, InteractionClaimCache,
    InteractionDispatcher, InteractionTransport,
};
use serde_json::json;
use tokio::{
    sync::mpsc,
    time::{Duration, Instant, advance, sleep},
};
use twilight_model::{
    application::interaction::Interaction,
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

#[path = "support/interaction_dispatch.rs"]
mod dispatch_support;
use dispatch_support::DispatchDatabase;

const DISPATCH_SOURCE: &str = include_str!("../src/discord_dispatch/dispatch_flow.rs");
const INTERACTION_CONSUMER_SOURCE: &str =
    include_str!("../src/discord_runtime/typed_ingress/interaction.rs");

#[derive(Clone, Copy)]
enum AckPlan {
    Immediate,
    Delay(Duration),
    FirstNeverThenImmediate,
}

struct TimedTransport {
    plan: AckPlan,
    attempts: AtomicUsize,
    completions: AtomicUsize,
}

impl TimedTransport {
    fn new(plan: AckPlan) -> Self {
        Self {
            plan,
            attempts: AtomicUsize::new(0),
            completions: AtomicUsize::new(0),
        }
    }
}

impl InteractionTransport for TimedTransport {
    fn acknowledge<'a>(
        &'a self,
        _id: Id<InteractionMarker>,
        _token: &'a str,
        _response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            match self.plan {
                AckPlan::Delay(delay) => sleep(delay).await,
                AckPlan::FirstNeverThenImmediate if attempt == 0 => {
                    std::future::pending::<()>().await;
                }
                AckPlan::Immediate | AckPlan::FirstNeverThenImmediate => {}
            }
            self.completions.fetch_add(1, Ordering::SeqCst);
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

fn dispatcher(
    transport: Arc<TimedTransport>,
) -> (
    DispatchDatabase,
    InteractionDispatcher<TimedTransport>,
    mpsc::Receiver<cdr_runtime::discord_dispatch::InboundInteractionWork>,
) {
    let database = DispatchDatabase::new();
    let (sender, receiver) = mpsc::channel(4);
    let dispatcher =
        InteractionDispatcher::new(transport, policy(), false, sender, database.path())
            .with_claim_cache(InteractionClaimCache::new(8));
    (database, dispatcher, receiver)
}

#[test]
fn idd_00_consumer_passes_the_original_receipt_instant_to_dispatch() {
    let consumer = INTERACTION_CONSUMER_SOURCE
        .split_whitespace()
        .collect::<String>();
    assert!(consumer.contains(".dispatch(&item.event,item.received_at,item.tag)"));
    assert!(!consumer.contains(".dispatch(&item.event,Instant::now()"));
    assert!(DISPATCH_SOURCE.contains("received_at.checked_add(INTERACTION_ACK_BUDGET)"));
}

#[tokio::test(start_paused = true)]
async fn idd_01_exactly_expired_receipt_never_starts_ack_or_enqueues_work() {
    assert_eq!(INTERACTION_ACK_BUDGET, Duration::from_millis(2_500));
    let transport = Arc::new(TimedTransport::new(AckPlan::Immediate));
    let (_database, dispatcher, mut receiver) = dispatcher(Arc::clone(&transport));
    let received_at = Instant::now();
    advance(INTERACTION_ACK_BUDGET).await;

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(701),
                received_at,
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::DeadlineExceeded
    );
    assert_eq!(transport.attempts.load(Ordering::SeqCst), 0);
    assert!(receiver.try_recv().is_err());

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(701),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    assert_eq!(receiver.recv().await.unwrap().interaction_id.get(), 701);
}

#[tokio::test(start_paused = true)]
async fn idd_02_ack_timeout_keeps_nonexecuted_custody_and_suppresses_retry() {
    let transport = Arc::new(TimedTransport::new(AckPlan::FirstNeverThenImmediate));
    let (database, dispatcher, mut receiver) = dispatcher(Arc::clone(&transport));

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(702),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::DeadlineExceeded
    );
    assert_eq!(transport.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(transport.completions.load(Ordering::SeqCst), 0);
    assert!(receiver.try_recv().is_err());

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(702),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Duplicate
    );
    assert_eq!(transport.attempts.load(Ordering::SeqCst), 1);
    assert!(receiver.try_recv().is_err());
    let held = cdr_store::ingress::by_origin(database.path(), 702)
        .unwrap()
        .unwrap();
    assert_eq!(held.state, "held");
    assert_eq!(held.hold_reason, "discord_ack_deadline");
}

#[tokio::test(start_paused = true)]
async fn idd_03_queue_delay_consumes_the_same_absolute_budget() {
    let transport = Arc::new(TimedTransport::new(AckPlan::Delay(Duration::from_millis(
        600,
    ))));
    let (_database, dispatcher, mut receiver) = dispatcher(Arc::clone(&transport));
    let received_at = Instant::now();
    advance(Duration::from_secs(2)).await;
    let dispatch_started_at = Instant::now();

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(703),
                received_at,
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::DeadlineExceeded
    );
    assert_eq!(
        Instant::now().duration_since(dispatch_started_at),
        Duration::from_millis(500)
    );
    assert_eq!(transport.completions.load(Ordering::SeqCst), 0);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn idd_04_ack_just_before_deadline_commits_and_enqueues_once() {
    let transport = Arc::new(TimedTransport::new(AckPlan::Delay(Duration::from_millis(
        2_499,
    ))));
    let (_database, dispatcher, mut receiver) = dispatcher(Arc::clone(&transport));

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(704),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    assert_eq!(transport.completions.load(Ordering::SeqCst), 1);
    assert_eq!(receiver.recv().await.unwrap().interaction_id.get(), 704);
}

#[tokio::test(start_paused = true)]
async fn idd_05_ack_ready_at_exact_deadline_fails_closed() {
    let transport = Arc::new(TimedTransport::new(AckPlan::Delay(INTERACTION_ACK_BUDGET)));
    let (_database, dispatcher, mut receiver) = dispatcher(Arc::clone(&transport));

    assert_eq!(
        dispatcher
            .dispatch(
                &interaction(705),
                Instant::now(),
                InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::DeadlineExceeded
    );
    assert_eq!(transport.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(transport.completions.load(Ordering::SeqCst), 0);
    assert!(receiver.try_recv().is_err());
}
