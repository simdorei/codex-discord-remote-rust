use tokio::sync::oneshot;

use super::{
    GatewayActivationError, GatewayRuntime,
    activation::{ActivationStopOutcome, ActivationWaitOutcome, ShardActivation},
    ingress::GatewayIngressConfig,
};

const ACTIVATION_SOURCE: &str = include_str!("gateway/activation.rs");

#[tokio::test]
async fn ga_15_stopped_gate_maps_to_the_public_typed_error() {
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");
    let receivers = runtime
        .take_ingress_receivers()
        .expect("receiver extraction succeeds");
    assert_eq!(
        runtime.activation.stop(),
        ActivationStopOutcome::StoppedBeforeActivation
    );

    assert_eq!(
        runtime.activate_typed_consumers().unwrap_err(),
        GatewayActivationError::Stopped
    );
    drop(receivers);
}

#[tokio::test]
async fn ga_16_double_typed_activation_emits_no_second_wake() {
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");
    let receivers = runtime
        .take_ingress_receivers()
        .expect("receiver extraction succeeds");
    let before = runtime.activation.notification_count();

    runtime
        .activate_typed_consumers()
        .expect("first activation succeeds");
    let after_first = runtime.activation.notification_count();
    assert_eq!(after_first, before + 1);
    assert_eq!(
        runtime.activate_typed_consumers().unwrap_err(),
        GatewayActivationError::AlreadyActivated
    );
    assert_eq!(runtime.activation.notification_count(), after_first);
    drop(receivers);
}

#[test]
fn ga_17_public_start_is_paused_and_has_no_eager_compatibility_path() {
    let paused = function_body(
        ACTIVATION_SOURCE,
        "pub async fn start_paused(",
        "/// Activates shard polling",
    );

    assert!(paused.contains("ShardActivation::paused()"));
    assert!(!paused.contains("legacy_activated"));
    assert!(!ACTIVATION_SOURCE.contains("pub async fn start("));
}

#[tokio::test]
async fn ga_18_activation_after_actual_notify_arming_has_no_lost_wake() {
    let activation = ShardActivation::paused();
    let waiter_activation = activation.clone();
    let (armed, armed_receiver) = oneshot::channel();
    let waiter = tokio::spawn(async move {
        waiter_activation
            .wait_with_arming_probe(move || {
                armed.send(()).expect("arming observer remains");
            })
            .await
    });
    armed_receiver
        .await
        .expect("waiter reports actual notification arming");

    activation.activate().expect("first activation succeeds");

    assert_eq!(
        waiter.await.expect("armed waiter exits"),
        ActivationWaitOutcome::Activated
    );
}

fn function_body<'a>(source: &'a str, signature: &str, next_item: &str) -> &'a str {
    let start = source.find(signature).expect("function signature exists");
    let end = source[start..]
        .find(next_item)
        .map(|offset| start + offset)
        .expect("following item bounds the function");
    &source[start..end]
}
