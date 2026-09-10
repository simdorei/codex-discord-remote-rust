use super::{
    ingress::{
        GatewayIngress, GatewayIngressConfig, GatewayIngressConfigError, IngressLane,
        ReceiveErrorPublishOutcome, UnavailableReason,
    },
    runtime_publication::{GatewayIngressPublishOutcomes, publish_receive_error},
};

fn config(capacity: usize) -> GatewayIngressConfig {
    GatewayIngressConfig {
        receive_error_capacity: capacity,
        ..GatewayIngressConfig::default()
    }
}

#[test]
fn gre_01_receive_error_lane_rejects_zero_capacity() {
    assert_eq!(
        GatewayIngress::new(config(0)).unwrap_err(),
        GatewayIngressConfigError::ZeroCapacity(IngressLane::ReceiveError)
    );
}

#[test]
fn gre_02_receive_error_lane_moves_exact_payload_and_reports_pressure() {
    let (ingress, mut receivers) = GatewayIngress::new(config(1)).unwrap();
    let outcomes = GatewayIngressPublishOutcomes::default();

    assert_eq!(
        publish_receive_error(3, "first decode failure".into(), &ingress, &outcomes),
        ReceiveErrorPublishOutcome::Accepted
    );
    assert_eq!(
        publish_receive_error(4, "second decode failure".into(), &ingress, &outcomes),
        ReceiveErrorPublishOutcome::Dropped {
            reason: UnavailableReason::Full,
        }
    );
    let received = receivers.receive_errors.try_recv().unwrap();
    assert_eq!(received.shard, 3);
    assert_eq!(received.message, "first decode failure");
    drop(receivers.receive_errors);
    assert_eq!(
        publish_receive_error(5, "closed decode failure".into(), &ingress, &outcomes),
        ReceiveErrorPublishOutcome::Dropped {
            reason: UnavailableReason::Closed,
        }
    );

    let snapshot = outcomes.snapshot();
    assert_eq!(snapshot.receive_errors_accepted, 1);
    assert_eq!(snapshot.receive_errors_dropped, 2);
}

#[test]
fn gre_03_receive_errors_do_not_spend_event_attempt_sequences() {
    let (ingress, mut receivers) = GatewayIngress::new_for_test(config(1), 70).unwrap();
    let outcomes = GatewayIngressPublishOutcomes::default();
    assert_eq!(
        publish_receive_error(6, "decode failure".into(), &ingress, &outcomes),
        ReceiveErrorPublishOutcome::Accepted
    );
    let _ = ingress.publish(
        super::ingress_tests::interaction_event(71),
        tokio::time::Instant::now(),
    );
    assert_eq!(
        receivers.normal_interactions.try_recv().unwrap().sequence,
        71
    );
}
