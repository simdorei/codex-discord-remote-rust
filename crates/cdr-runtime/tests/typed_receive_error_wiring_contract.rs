const GATEWAY: &str = include_str!("../../cdr-discord/src/gateway.rs");
const ACTIVATION: &str = include_str!("../../cdr-discord/src/gateway/activation.rs");
const SHARD: &str = include_str!("../../cdr-discord/src/gateway/shard.rs");
const PUBLICATION: &str = include_str!("../../cdr-discord/src/gateway/runtime_publication.rs");
const TYPES: &str = include_str!("../../cdr-discord/src/gateway/ingress/types.rs");
const SPAWN: &str = include_str!("../src/discord_runtime/typed_ingress/spawn.rs");
const RECEIVE_ERROR: &str = include_str!("../src/discord_runtime/typed_ingress/receive_error.rs");
const LOOP: &str = include_str!("../src/discord_runtime/gateway_loop.rs");

fn compact(source: &str) -> String {
    source.split_whitespace().collect()
}

#[test]
fn tre_01_legacy_event_broadcast_and_event_clone_are_absent() {
    for forbidden in [
        "GatewayItem",
        "events: broadcast::Sender",
        "receiver: broadcast::Receiver",
        "pub fn subscribe(&self)",
        "pub async fn next(&mut self)",
    ] {
        assert!(
            !GATEWAY.contains(forbidden),
            "legacy marker remains: {forbidden}"
        );
    }
    assert!(!ACTIVATION.contains("legacy_activated"));
    assert!(!SHARD.contains("broadcast::Sender"));
    assert!(!PUBLICATION.contains("event.clone()"));
    assert!(!PUBLICATION.contains("legacy"));
}

#[test]
fn tre_02_receive_errors_have_one_owned_typed_lane_and_consumer() {
    let types = compact(TYPES);
    let spawn = compact(SPAWN);
    assert!(types.contains("pubstructReceiveErrorIngress{pubshard:u32,pubmessage:String,"));
    assert!(types.contains("pubreceive_errors:mpsc::Receiver<ReceiveErrorIngress>"));
    assert!(spawn.contains("receive_error::run(receive_errors,shutdown)"));
    assert!(RECEIVE_ERROR.contains("Discord gateway receive error on shard"));
    assert!(!LOOP.contains("GatewayItem"));
    assert!(!LOOP.contains("gateway.next()"));
}
