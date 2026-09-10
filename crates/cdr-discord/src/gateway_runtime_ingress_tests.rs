use super::{
    GatewayIngressReceiversError, GatewayRuntime, GatewayStartError,
    ingress::{GatewayIngressConfig, GatewayIngressConfigError, IngressLane},
    ingress_tests::config,
};

#[tokio::test]
async fn gi_rt_01_runtime_owns_default_receivers_until_exactly_one_extraction() {
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");

    let receivers = runtime
        .take_ingress_receivers()
        .expect("first extraction transfers the owned receivers");
    assert_eq!(receivers.normal_interactions.max_capacity(), 64);
    assert_eq!(receivers.reserved_interactions.max_capacity(), 4);
    assert_eq!(receivers.messages.max_capacity(), 1_024);
    assert_eq!(
        runtime.take_ingress_receivers().unwrap_err(),
        GatewayIngressReceiversError::AlreadyTaken
    );
}

#[test]
fn gi_rt_07_invalid_runtime_ingress_config_is_typed_and_fail_closed() {
    assert!(matches!(
        GatewayRuntime::new_offline_for_test(config(0, 1, 1)),
        Err(GatewayStartError::IngressConfig(
            GatewayIngressConfigError::ZeroCapacity(IngressLane::NormalInteraction)
        ))
    ));
}

#[tokio::test]
async fn gi_rt_08_message_gap_subscription_is_available_before_typed_activation() {
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");
    let gaps = runtime.subscribe_message_gaps();
    assert!(gaps.snapshot().unwrap().is_empty());
    let _receivers = runtime.take_ingress_receivers().unwrap();
    runtime.activate_typed_consumers().unwrap();
}
