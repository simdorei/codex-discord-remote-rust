use std::cell::Cell;

use cdr_discord::gateway::ingress::{
    GatewayIngress, GatewayIngressConfig, MessageGapAckOutcome, MessageGapFenceError,
    PublishOutcome,
};
use tokio::time::Instant;
use twilight_model::channel::Message;
use twilight_model::gateway::event::Event;
use twilight_model::gateway::payload::incoming::MessageCreate;
use twilight_model::id::{Id, marker::ChannelMarker};

use super::channel_has_pending_gap;

fn message(id: u64, channel_id: u64) -> Message {
    serde_json::from_value(serde_json::json!({
        "attachments": [],
        "author": {"avatar": null, "bot": false, "discriminator": "0001",
            "id": "3", "username": "tester"},
        "channel_id": channel_id.to_string(), "content": "hello",
        "edited_timestamp": null, "embeds": [], "id": id.to_string(),
        "mention_everyone": false, "mention_roles": [], "mentions": [],
        "pinned": false, "timestamp": "2020-02-02T02:02:02.020000+00:00",
        "tts": false, "type": 0
    }))
    .expect("valid message")
}

#[test]
fn tic_01_pending_gap_fence_is_channel_specific_and_clears_only_after_ack() {
    let (ingress, receivers) =
        GatewayIngress::new(GatewayIngressConfig::default()).expect("valid ingress");
    let gaps = ingress.subscribe_message_gaps();
    drop(receivers.messages);
    assert!(matches!(
        ingress.publish(
            Event::MessageCreate(Box::new(MessageCreate(message(41, 7)))),
            Instant::now(),
        ),
        PublishOutcome::MessageRecoverableGap { .. }
    ));

    assert!(channel_has_pending_gap(&gaps, 7).expect("gap snapshot"));
    assert!(!channel_has_pending_gap(&gaps, 8).expect("other channel snapshot"));
    let notice = gaps
        .snapshot()
        .expect("gap snapshot")
        .pop()
        .expect("one pending gap");
    assert_eq!(
        gaps.acknowledge(notice).expect("acknowledge gap"),
        MessageGapAckOutcome::Cleared
    );
    assert!(!channel_has_pending_gap(&gaps, 7).expect("cleared snapshot"));
}

#[test]
fn tic_02_gap_recorded_after_clean_snapshot_blocks_the_discard_claim() {
    let (ingress, receivers) =
        GatewayIngress::new(GatewayIngressConfig::default()).expect("valid ingress");
    let gaps = ingress.subscribe_message_gaps();
    let channel_id = Id::<ChannelMarker>::new(7);
    let fence = gaps
        .capture_clear_fence(channel_id)
        .expect("capture clean gap state")
        .expect("channel has no pending gap");

    drop(receivers.messages);
    assert!(matches!(
        ingress.publish(
            Event::MessageCreate(Box::new(MessageCreate(message(42, 7)))),
            Instant::now(),
        ),
        PublishOutcome::MessageRecoverableGap { .. }
    ));

    let claim_ran = Cell::new(false);
    assert_eq!(
        gaps.with_current_fence(&fence, || claim_ran.set(true)),
        Err(MessageGapFenceError::Advanced { channel_id })
    );
    assert!(!claim_ran.get());
}
