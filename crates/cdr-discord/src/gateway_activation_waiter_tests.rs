use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{sync::mpsc, task::JoinHandle, time::timeout};

use super::{
    activation::{ActivationStopOutcome, ActivationWaitOutcome, ShardActivation},
    shard::run_after_activation,
};

const WAITER_COUNT: usize = 8;
const JOIN_TIMEOUT: Duration = Duration::from_millis(250);

#[tokio::test]
async fn ga_13_activation_releases_every_registered_waiter() {
    let activation = ShardActivation::paused();
    let (registered, mut registrations) = mpsc::unbounded_channel();
    let waiters = (0..WAITER_COUNT)
        .map(|_| {
            let activation = activation.clone();
            let registered = registered.clone();
            tokio::spawn(async move {
                registered.send(()).expect("registration observer remains");
                activation.wait().await
            })
        })
        .collect::<Vec<_>>();
    drop(registered);
    await_registrations(&mut registrations).await;

    activation.activate().expect("first activation succeeds");

    await_outcomes(waiters, ActivationWaitOutcome::Activated).await;
}

#[tokio::test]
async fn ga_14_stop_releases_every_registered_waiter_without_running() {
    let activation = ShardActivation::paused();
    let active_entries = Arc::new(AtomicUsize::new(0));
    let (registered, mut registrations) = mpsc::unbounded_channel();
    let waiters = (0..WAITER_COUNT)
        .map(|_| {
            let activation = activation.clone();
            let active_entries = Arc::clone(&active_entries);
            let registered = registered.clone();
            tokio::spawn(async move {
                registered.send(()).expect("registration observer remains");
                run_after_activation(&activation, || async move {
                    active_entries.fetch_add(1, Ordering::AcqRel);
                })
                .await;
                activation.wait().await
            })
        })
        .collect::<Vec<_>>();
    drop(registered);
    await_registrations(&mut registrations).await;

    assert_eq!(
        activation.stop(),
        ActivationStopOutcome::StoppedBeforeActivation
    );

    await_outcomes(waiters, ActivationWaitOutcome::Stopped).await;
    assert_eq!(active_entries.load(Ordering::Acquire), 0);
}

async fn await_registrations(registrations: &mut mpsc::UnboundedReceiver<()>) {
    for _ in 0..WAITER_COUNT {
        registrations
            .recv()
            .await
            .expect("every waiter announces registration");
    }
}

async fn await_outcomes(
    waiters: Vec<JoinHandle<ActivationWaitOutcome>>,
    expected: ActivationWaitOutcome,
) {
    timeout(JOIN_TIMEOUT, async move {
        for waiter in waiters {
            assert_eq!(waiter.await.expect("waiter exits normally"), expected);
        }
    })
    .await
    .expect("every registered waiter exits within the bound");
}
