use cdr_discord::idempotent_message::message_nonce;
use cdr_runtime::message_worker::reply_delivery::{
    MESSAGE_ERROR_DOMAIN, MESSAGE_REPLY_DOMAIN, MessageReplyIdentity, MessageReplyKind,
};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

#[test]
fn retry_reuses_the_same_inbound_reply_nonce_and_chunks_remain_distinct() {
    let source = Id::<MessageMarker>::new(42);
    let channel = Id::<ChannelMarker>::new(7);
    let identity = MessageReplyIdentity::new(source, MessageReplyKind::ActionResult);

    let first_attempt = message_nonce(identity.domain(), channel, identity.logical_key(), 1);
    let retry_attempt = message_nonce(identity.domain(), channel, identity.logical_key(), 1);
    let other_chunk = message_nonce(identity.domain(), channel, identity.logical_key(), 2);

    assert_eq!(identity.domain(), MESSAGE_REPLY_DOMAIN);
    assert_eq!(identity.logical_key(), "inbound-message/42/action-result");
    assert_eq!(first_attempt, retry_attempt);
    assert_ne!(first_attempt, other_chunk);
}

#[test]
fn one_inbound_message_has_honestly_separated_reply_categories() {
    let source = Id::<MessageMarker>::new(42);
    let kinds = [
        MessageReplyKind::PendingConfirmation,
        MessageReplyKind::PlannedResponse,
        MessageReplyKind::ActionResult,
        MessageReplyKind::ErrorReport,
    ];
    let keys = kinds.map(|kind| {
        MessageReplyIdentity::new(source, kind)
            .logical_key()
            .to_owned()
    });

    assert_eq!(
        keys,
        [
            "inbound-message/42/pending-confirmation",
            "inbound-message/42/planned-response",
            "inbound-message/42/action-result",
            "inbound-message/42/error-report",
        ]
    );
    assert_ne!(MESSAGE_REPLY_DOMAIN, MESSAGE_ERROR_DOMAIN);
    let channel = Id::<ChannelMarker>::new(7);
    let nonces = kinds.map(|kind| {
        let identity = MessageReplyIdentity::new(source, kind);
        message_nonce(identity.domain(), channel, identity.logical_key(), 0)
    });
    for (index, nonce) in nonces.iter().enumerate() {
        assert!(!nonces[..index].contains(nonce));
    }
}

#[test]
fn gateway_error_identity_uses_the_source_message_id_and_a_separate_domain() {
    let first =
        MessageReplyIdentity::new(Id::<MessageMarker>::new(42), MessageReplyKind::ErrorReport);
    let second =
        MessageReplyIdentity::new(Id::<MessageMarker>::new(43), MessageReplyKind::ErrorReport);

    assert_eq!(first.domain(), MESSAGE_ERROR_DOMAIN);
    assert_eq!(first.logical_key(), "inbound-message/42/error-report");
    assert_eq!(second.logical_key(), "inbound-message/43/error-report");
    assert_ne!(first, second);
}
