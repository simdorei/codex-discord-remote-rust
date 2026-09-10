use cdr_discord::{
    components::busy_button_row,
    idempotent_message::{
        idempotent_message_request, idempotent_message_request_with_components, message_nonce,
    },
};
use serde_json::Value;
use twilight_http::request::Method;
use twilight_model::channel::message::AllowedMentions;
use twilight_model::id::{Id, marker::ChannelMarker};

const ASSISTANT_DOMAIN: &str = "session-mirror/assistant-text/v1";

#[test]
fn request_payload_enforces_discord_nonce_deduplication_and_disables_mentions() {
    let channel = Id::<ChannelMarker>::new(42);
    let request = idempotent_message_request(
        channel,
        "hello @everyone",
        ASSISTANT_DOMAIN,
        "thread-1:abc",
        0,
    )
    .unwrap();

    assert_eq!(request.path(), "channels/42/messages");
    assert_eq!(request.method(), Method::Post);
    let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 4);
    assert_eq!(body["content"], "hello @everyone");
    assert_eq!(
        body["allowed_mentions"],
        serde_json::to_value(AllowedMentions::default()).unwrap()
    );
    assert_eq!(body["enforce_nonce"], true);
    assert_eq!(body["nonce"].as_u64(), Some(2_340_874_901_045_434_203));
}

#[test]
fn component_request_preserves_buttons_without_weakening_nonce_enforcement() {
    let channel = Id::<ChannelMarker>::new(42);
    let components = vec![busy_button_row("0123456789abcdef01234567", true).unwrap()];
    let request = idempotent_message_request_with_components(
        channel,
        "Choose an action",
        &components,
        "message/reply/v1",
        "source-message:99",
        0,
    )
    .unwrap();

    let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 5);
    assert_eq!(
        body["components"],
        serde_json::to_value(&components).unwrap()
    );
    assert_eq!(body["enforce_nonce"], true);
    assert_eq!(
        body["allowed_mentions"],
        serde_json::to_value(AllowedMentions::default()).unwrap()
    );
}

#[test]
fn nonce_stays_within_discord_signed_i64_range() {
    let nonce: u64 = message_nonce(
        "completion/v1",
        Id::<ChannelMarker>::new(42),
        "high-bit-0",
        0,
    );

    assert_eq!(nonce, 3_227_115_366_091_472_613);
    assert!(i64::try_from(nonce).is_ok());
}

#[test]
fn nonce_is_stable_for_a_retry_and_separates_every_logical_dimension() {
    let channel = Id::<ChannelMarker>::new(42);
    let same = message_nonce(ASSISTANT_DOMAIN, channel, "thread-1:abc", 0);

    assert_eq!(
        same,
        message_nonce(ASSISTANT_DOMAIN, channel, "thread-1:abc", 0)
    );
    assert_ne!(
        same,
        message_nonce("completion/v1", channel, "thread-1:abc", 0)
    );
    assert_ne!(
        same,
        message_nonce(ASSISTANT_DOMAIN, channel, "thread-1:def", 0)
    );
    assert_ne!(
        same,
        message_nonce(ASSISTANT_DOMAIN, channel, "thread-1:abc", 1)
    );
    assert_ne!(
        same,
        message_nonce(
            ASSISTANT_DOMAIN,
            Id::<ChannelMarker>::new(43),
            "thread-1:abc",
            0,
        )
    );
}

#[test]
fn raw_json_path_keeps_the_existing_discord_content_limit_validation() {
    let channel = Id::<ChannelMarker>::new(42);

    assert!(
        idempotent_message_request(channel, &"x".repeat(1_900), ASSISTANT_DOMAIN, "key", 0).is_ok()
    );
    assert!(
        idempotent_message_request(channel, &"x".repeat(1_901), ASSISTANT_DOMAIN, "key", 0)
            .is_err()
    );
    assert!(idempotent_message_request(channel, "", ASSISTANT_DOMAIN, "key", 0).is_err());
    assert!(idempotent_message_request(channel, "   ", ASSISTANT_DOMAIN, "key", 0).is_err());
}
