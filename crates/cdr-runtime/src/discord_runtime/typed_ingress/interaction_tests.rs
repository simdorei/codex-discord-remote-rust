use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::json;
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::time::{Duration, Instant, timeout};
use twilight_model::gateway::payload::incoming::InteractionCreate;

use super::*;

#[derive(Clone)]
struct ProbeHandler {
    started: mpsc::UnboundedSender<(u64, InteractionIngressTag)>,
    release: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

impl InteractionHandler for ProbeHandler {
    fn handle(&self, item: InteractionIngress) -> InteractionFuture {
        let probe = self.clone();
        Box::pin(async move {
            let active = probe.active.fetch_add(1, Ordering::SeqCst) + 1;
            probe.maximum.fetch_max(active, Ordering::SeqCst);
            probe.started.send((item.sequence, item.tag)).unwrap();
            let permit = Arc::clone(&probe.release).acquire_owned().await.unwrap();
            permit.forget();
            probe.active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

fn probe() -> (
    ProbeHandler,
    mpsc::UnboundedReceiver<(u64, InteractionIngressTag)>,
) {
    let (started, receiver) = mpsc::unbounded_channel();
    (
        ProbeHandler {
            started,
            release: Arc::new(Semaphore::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
            maximum: Arc::new(AtomicUsize::new(0)),
        },
        receiver,
    )
}

pub(super) fn item(sequence: u64, tag: InteractionIngressTag) -> InteractionIngress {
    let event: InteractionCreate = serde_json::from_value(json!({
        "application_id":"2",
        "authorizing_integration_owners":{},
        "id":sequence.to_string(),
        "token":format!("token-{sequence}"),
        "type":1
    }))
    .unwrap();
    InteractionIngress {
        sequence,
        received_at: Instant::now(),
        tag,
        event: Box::new(event),
    }
}

pub(super) fn filled_lane(
    count: u64,
    tag: InteractionIngressTag,
) -> (
    mpsc::Sender<InteractionIngress>,
    mpsc::Receiver<InteractionIngress>,
) {
    let (sender, receiver) = mpsc::channel(32);
    for sequence in 1..=count {
        sender.try_send(item(sequence, tag)).unwrap();
    }
    (sender, receiver)
}

async fn receive_started(
    receiver: &mut mpsc::UnboundedReceiver<(u64, InteractionIngressTag)>,
    count: usize,
) -> Vec<(u64, InteractionIngressTag)> {
    let mut started = Vec::with_capacity(count);
    for _ in 0..count {
        started.push(
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .expect("interaction starts")
                .expect("probe remains open"),
        );
    }
    started
}

#[tokio::test]
async fn tic_lane_01_normal_never_exceeds_sixteen_active_dispatches() {
    let (_input, receiver) = filled_lane(17, InteractionIngressTag::Normal);
    let (handler, mut started) = probe();
    let release = Arc::clone(&handler.release);
    let maximum = Arc::clone(&handler.maximum);
    let (shutdown, shutdown_rx) = watch::channel(false);
    let runner = tokio::spawn(run_lane(
        receiver,
        InteractionLane::Normal,
        NORMAL_CONCURRENCY,
        handler,
        shutdown_rx,
    ));

    assert_eq!(receive_started(&mut started, 16).await.len(), 16);
    assert!(started.try_recv().is_err());
    assert_eq!(maximum.load(Ordering::SeqCst), 16);
    release.add_permits(16);
    assert_eq!(receive_started(&mut started, 1).await[0].0, 17);
    release.add_permits(1);
    shutdown.send(true).unwrap();
    runner.await.unwrap().unwrap();
}

#[tokio::test]
async fn tic_lane_02_reserved_four_progress_while_normal_sixteen_are_blocked() {
    let (normal_input, normal_receiver) = filled_lane(17, InteractionIngressTag::Normal);
    let (reserved_input, reserved_receiver) = filled_lane(5, InteractionIngressTag::Busy);
    let (normal_handler, mut normal_started) = probe();
    let (reserved_handler, mut reserved_started) = probe();
    let normal_release = Arc::clone(&normal_handler.release);
    let reserved_release = Arc::clone(&reserved_handler.release);
    let reserved_maximum = Arc::clone(&reserved_handler.maximum);
    let (shutdown, shutdown_rx) = watch::channel(false);
    let normal = tokio::spawn(run_lane(
        normal_receiver,
        InteractionLane::Normal,
        NORMAL_CONCURRENCY,
        normal_handler,
        shutdown_rx.clone(),
    ));
    let reserved = tokio::spawn(run_lane(
        reserved_receiver,
        InteractionLane::Reserved,
        RESERVED_CONCURRENCY,
        reserved_handler,
        shutdown_rx,
    ));

    assert_eq!(receive_started(&mut normal_started, 16).await.len(), 16);
    assert_eq!(receive_started(&mut reserved_started, 4).await.len(), 4);
    assert!(normal_started.try_recv().is_err());
    assert!(reserved_started.try_recv().is_err());
    assert_eq!(reserved_maximum.load(Ordering::SeqCst), 4);

    normal_release.add_permits(16);
    reserved_release.add_permits(4);
    assert_eq!(receive_started(&mut normal_started, 1).await[0].0, 17);
    assert_eq!(receive_started(&mut reserved_started, 1).await[0].0, 5);
    normal_release.add_permits(1);
    reserved_release.add_permits(1);
    drop(normal_input);
    drop(reserved_input);
    shutdown.send(true).unwrap();
    normal.await.unwrap().unwrap();
    reserved.await.unwrap().unwrap();
}

#[tokio::test]
async fn tic_lane_03_wrong_tag_fails_closed_instead_of_crossing_lanes() {
    let (_input, receiver) = filled_lane(1, InteractionIngressTag::Busy);
    let (handler, _started) = probe();
    let (_shutdown, shutdown_rx) = watch::channel(false);

    let error = run_lane(
        receiver,
        InteractionLane::Normal,
        NORMAL_CONCURRENCY,
        handler,
        shutdown_rx,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        DiscordRuntimeError::TypedInteractionTag {
            lane: "normal-interaction",
            tag: InteractionIngressTag::Busy,
        }
    ));
}

#[tokio::test]
async fn tic_lane_04_reserved_drains_stopping_items_after_shutdown_begins() {
    let (input, receiver) = mpsc::channel(1);
    let (handler, mut started) = probe();
    let release = Arc::clone(&handler.release);
    let (shutdown, shutdown_rx) = watch::channel(true);
    let runner = tokio::spawn(run_lane(
        receiver,
        InteractionLane::Reserved,
        RESERVED_CONCURRENCY,
        handler,
        shutdown_rx,
    ));
    tokio::task::yield_now().await;

    input
        .send(item(40, InteractionIngressTag::Stopping))
        .await
        .expect("reserved receiver stays open during bounded shutdown drain");
    drop(input);
    assert_eq!(
        receive_started(&mut started, 1).await,
        vec![(40, InteractionIngressTag::Stopping)]
    );
    release.add_permits(1);
    runner.await.unwrap().unwrap();
    drop(shutdown);
}

#[tokio::test(start_paused = true)]
async fn tic_lane_05_reserved_shutdown_drain_has_a_hard_deadline() {
    let (_input, receiver) = filled_lane(1, InteractionIngressTag::Stopping);
    let (handler, mut started) = probe();
    let (_shutdown, shutdown_rx) = watch::channel(true);
    let runner = tokio::spawn(run_lane(
        receiver,
        InteractionLane::Reserved,
        RESERVED_CONCURRENCY,
        handler,
        shutdown_rx,
    ));

    assert_eq!(started.recv().await.unwrap().0, 1);
    tokio::time::advance(RESERVED_SHUTDOWN_DRAIN_TIMEOUT).await;
    tokio::task::yield_now().await;
    assert!(matches!(
        runner.await.unwrap().unwrap_err(),
        DiscordRuntimeError::TypedIngressDrainTimeout("reserved-interaction")
    ));
}
