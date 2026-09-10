use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use super::{
    GatewayActivationError, GatewayRuntime,
    activation::{ActivationStopOutcome, ActivationWaitOutcome, ShardActivation},
    ingress::GatewayIngressConfig,
    shard::run_after_activation,
    shutdown::GatewayTask,
};

const ACTIVATION_SOURCE: &str = include_str!("gateway/activation.rs");
const SHARD_SOURCE: &str = include_str!("gateway/shard.rs");

#[tokio::test]
async fn ga_01_paused_shard_surrogate_cannot_cross_poll_marker() {
    let activation = ShardActivation::paused();
    let crossed_poll_marker = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&crossed_poll_marker);
    let task_activation = activation.clone();
    let task = tokio::spawn(async move {
        run_after_activation(&task_activation, || async move {
            marker.store(true, Ordering::Release);
        })
        .await;
    });

    tokio::task::yield_now().await;
    assert!(!crossed_poll_marker.load(Ordering::Acquire));

    activation.activate().expect("first activation succeeds");
    task.await.expect("surrogate exits after activation");
    assert!(crossed_poll_marker.load(Ordering::Acquire));
}

#[tokio::test]
async fn ga_02_activation_before_waiter_registration_has_no_lost_wake() {
    let activation = ShardActivation::paused();
    activation.activate().expect("first activation succeeds");

    assert_eq!(activation.wait().await, ActivationWaitOutcome::Activated);
}

#[tokio::test]
async fn ga_03_activation_after_waiter_registration_has_no_lost_wake() {
    let activation = ShardActivation::paused();
    let waiter_activation = activation.clone();
    let waiter = tokio::spawn(async move { waiter_activation.wait().await });
    tokio::task::yield_now().await;

    activation.activate().expect("first activation succeeds");

    assert_eq!(
        waiter.await.expect("registered waiter exits"),
        ActivationWaitOutcome::Activated
    );
}

#[tokio::test]
async fn ga_04_cancelled_waiter_cannot_consume_a_future_activation() {
    let activation = ShardActivation::paused();
    let cancelled_activation = activation.clone();
    let cancelled = tokio::spawn(async move { cancelled_activation.wait().await });
    tokio::task::yield_now().await;
    cancelled.abort();
    let _ = cancelled.await;

    activation.activate().expect("activation remains available");
    assert_eq!(activation.wait().await, ActivationWaitOutcome::Activated);
}

#[tokio::test]
async fn ga_05_typed_activation_refuses_runtime_owned_receivers() {
    let runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");

    assert_eq!(
        runtime.activate_typed_consumers().unwrap_err(),
        GatewayActivationError::IngressReceiversNotTaken
    );
    assert!(runtime.activation.is_paused());
}

#[tokio::test]
async fn ga_06_one_extraction_then_one_typed_activation_succeeds() {
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");
    let waiter_activation = runtime.activation.clone();
    let waiter = tokio::spawn(async move { waiter_activation.wait().await });

    let receivers = runtime
        .take_ingress_receivers()
        .expect("typed consumers take all three lanes once");
    runtime
        .activate_typed_consumers()
        .expect("taken receivers permit typed activation");

    assert_eq!(
        waiter.await.expect("paused waiter exits"),
        ActivationWaitOutcome::Activated
    );
    drop(receivers);
}

#[tokio::test]
async fn ga_07_double_typed_activation_is_a_typed_no_op() {
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");
    let receivers = runtime
        .take_ingress_receivers()
        .expect("first receiver extraction succeeds");
    runtime
        .activate_typed_consumers()
        .expect("first activation succeeds");

    assert_eq!(
        runtime.activate_typed_consumers().unwrap_err(),
        GatewayActivationError::AlreadyActivated
    );
    assert!(runtime.activation.is_activated());
    drop(receivers);
}

#[tokio::test]
async fn ga_08_explicit_shutdown_releases_paused_waiters_without_polling() {
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");
    let crossed_poll_marker = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&crossed_poll_marker);
    let activation = runtime.activation.clone();
    runtime.tasks.push(GatewayTask::new(
        0,
        tokio::spawn(async move {
            run_after_activation(&activation, || async move {
                marker.store(true, Ordering::Release);
            })
            .await;
        }),
    ));
    tokio::task::yield_now().await;

    runtime.shutdown().await.expect("paused shutdown completes");

    assert!(!crossed_poll_marker.load(Ordering::Acquire));
}

#[tokio::test]
async fn ga_09_drop_terminates_paused_waiters_without_polling() {
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");
    let crossed_poll_marker = Arc::new(AtomicBool::new(false));
    let future_dropped = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&crossed_poll_marker);
    let dropped = Arc::clone(&future_dropped);
    let activation = runtime.activation.clone();
    runtime.tasks.push(GatewayTask::new(
        0,
        tokio::spawn(async move {
            let _guard = Dropped(dropped);
            run_after_activation(&activation, || async move {
                marker.store(true, Ordering::Release);
            })
            .await;
        }),
    ));
    tokio::task::yield_now().await;

    drop(runtime);
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    assert!(future_dropped.load(Ordering::Acquire));
    assert!(!crossed_poll_marker.load(Ordering::Acquire));
}

#[test]
fn ga_10_activation_and_stop_have_one_atomic_linearization_winner() {
    for _ in 0..128 {
        let activation = ShardActivation::paused();
        let barrier = Arc::new(Barrier::new(3));
        let activate = spawn_at_barrier(&activation, &barrier, |gate| gate.activate());
        let stop = spawn_at_barrier(&activation, &barrier, |gate| gate.stop());
        barrier.wait();

        let activate = activate.join().expect("activate racer exits");
        let stop = stop.join().expect("stop racer exits");
        match stop {
            ActivationStopOutcome::StoppedBeforeActivation => {
                assert_eq!(
                    activate.unwrap_err().to_string(),
                    "gateway activation was stopped"
                );
            }
            ActivationStopOutcome::StoppedAfterActivation => {
                activate.expect("activation linearized before stop");
            }
            ActivationStopOutcome::AlreadyStopped => panic!("only one stop racer exists"),
        }
    }
}

fn spawn_at_barrier<T, F>(
    activation: &ShardActivation,
    barrier: &Arc<Barrier>,
    operation: F,
) -> thread::JoinHandle<T>
where
    T: Send + 'static,
    F: FnOnce(ShardActivation) -> T + Send + 'static,
{
    let activation = activation.clone();
    let barrier = Arc::clone(barrier);
    thread::spawn(move || {
        barrier.wait();
        operation(activation)
    })
}

#[test]
fn ga_11_production_shard_entry_is_gated_before_next_event() {
    let gate = SHARD_SOURCE
        .find("run_after_activation(&activation")
        .expect("production shard uses the injectable entry seam");
    let poll = SHARD_SOURCE
        .find("shard.next_event(gateway_event_flags())")
        .expect("production shard has one gateway poll point");

    assert!(gate < poll);
    assert_eq!(SHARD_SOURCE.matches("shard.next_event(").count(), 1);
}

#[test]
fn ga_12_only_consumer_gated_typed_start_path_remains() {
    assert!(ACTIVATION_SOURCE.contains("pub async fn start_paused("));
    assert!(ACTIVATION_SOURCE.contains("ShardActivation::paused()"));
    assert!(!ACTIVATION_SOURCE.contains("legacy_activated"));
}
