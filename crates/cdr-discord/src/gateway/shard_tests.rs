use tokio::sync::mpsc;

use super::ShardExitGuard;

#[test]
fn message_nonce_survives_decode_and_typed_ingress_as_number_or_string() {
    use crate::gateway::{
        ingress::{GatewayIngress, GatewayIngressConfig},
        ingress_tests::message_event,
        runtime_publication::{GatewayIngressPublishOutcomes, publish_decoded_event_with_nonce},
    };
    let twilight_gateway::Event::MessageCreate(message) = message_event(123) else {
        panic!("message fixture");
    };
    let (ingress, mut receivers) = GatewayIngress::new(GatewayIngressConfig::default()).unwrap();
    let outcomes = GatewayIngressPublishOutcomes::default();
    for nonce in [
        serde_json::json!(9_223_372_036_854_775_807_u64),
        serde_json::json!("9223372036854775807"),
    ] {
        let mut payload = serde_json::json!({"op":0,"s":1,"t":"MESSAGE_CREATE","d":message});
        payload["d"]["nonce"] = nonce;
        let (event, nonce) = super::decode_with_nonce(payload.to_string())
            .unwrap()
            .unwrap();
        publish_decoded_event_with_nonce(
            event,
            nonce,
            tokio::time::Instant::now(),
            &ingress,
            &outcomes,
        );
        let received = receivers.messages.try_recv().unwrap();
        assert_eq!(received.event.id.get(), 123);
        assert_eq!(received.nonce, Some(9_223_372_036_854_775_807));
    }
    assert_eq!(outcomes.snapshot().messages_accepted, 2);
}

#[test]
fn missing_or_invalid_nonce_does_not_drop_the_message() {
    let twilight_gateway::Event::MessageCreate(message) =
        crate::gateway::ingress_tests::message_event(123)
    else {
        panic!("message fixture");
    };
    for nonce in [
        serde_json::Value::Null,
        serde_json::json!(-1),
        serde_json::json!("invalid"),
    ] {
        let mut payload = serde_json::json!({"op":0,"s":1,"t":"MESSAGE_CREATE","d":message});
        payload["d"]["nonce"] = nonce;
        let (event, nonce) = super::decode_with_nonce(payload.to_string())
            .unwrap()
            .unwrap();
        assert!(matches!(event, twilight_gateway::Event::MessageCreate(_)));
        assert_eq!(nonce, None);
    }
}

#[tokio::test]
async fn gse_01_normal_task_exit_publishes_exact_shard() {
    let (sender, mut exits) = mpsc::channel(1);
    drop(ShardExitGuard { shard: 7, sender });

    assert_eq!(exits.recv().await, Some(7));
}

#[tokio::test]
async fn gse_02_panicking_task_publishes_before_join_reports_panic() {
    let (sender, mut exits) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        let _exit = ShardExitGuard { shard: 11, sender };
        panic!("injected shard panic");
    });

    assert_eq!(exits.recv().await, Some(11));
    assert!(task.await.unwrap_err().is_panic());
}
