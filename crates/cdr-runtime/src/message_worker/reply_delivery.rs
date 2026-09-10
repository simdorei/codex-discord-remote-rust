use std::future::Future;

use crate::completion_worker::{
    CompletionWorkerError, IdempotentChunk, send_recorded_message_with_components,
};
use cdr_discord::delivery::{DeliveryFailure, DeliveryPolicy, deliver_chunks_indexed};
use cdr_discord::http::DiscordHttp;
use cdr_discord::text::{split_delivery_chunks, split_exact_delivery_chunks};
use twilight_model::{
    channel::message::Component,
    id::{
        Id,
        marker::{ChannelMarker, MessageMarker},
    },
};

pub const MESSAGE_REPLY_DOMAIN: &str = "message/reply/v1";
pub const MESSAGE_ERROR_DOMAIN: &str = "message/error/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageReplyKind {
    PendingConfirmation,
    PlannedResponse,
    ActionResult,
    SavedRequest,
    ErrorReport,
}

impl MessageReplyKind {
    const fn domain(self) -> &'static str {
        match self {
            Self::ErrorReport => MESSAGE_ERROR_DOMAIN,
            Self::PendingConfirmation
            | Self::PlannedResponse
            | Self::ActionResult
            | Self::SavedRequest => MESSAGE_REPLY_DOMAIN,
        }
    }

    const fn key_segment(self) -> &'static str {
        match self {
            Self::PendingConfirmation => "pending-confirmation",
            Self::PlannedResponse => "planned-response",
            Self::ActionResult | Self::SavedRequest => "action-result",
            Self::ErrorReport => "error-report",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageReplyIdentity {
    domain: &'static str,
    logical_key: String,
}

impl MessageReplyIdentity {
    #[must_use]
    pub fn new(source_message_id: Id<MessageMarker>, kind: MessageReplyKind) -> Self {
        Self {
            domain: kind.domain(),
            logical_key: format!(
                "inbound-message/{}/{kind}",
                source_message_id.get(),
                kind = kind.key_segment()
            ),
        }
    }

    #[must_use]
    pub const fn domain(&self) -> &'static str {
        self.domain
    }

    #[must_use]
    pub fn logical_key(&self) -> &str {
        &self.logical_key
    }
}

pub async fn deliver_reply_text(
    db: &std::path::Path,
    api: &DiscordHttp,
    channel_id: Id<ChannelMarker>,
    source_message_id: Id<MessageMarker>,
    kind: MessageReplyKind,
    text: &str,
) -> Result<usize, DeliveryFailure<CompletionWorkerError>> {
    deliver_reply_text_with(
        source_message_id,
        kind,
        text,
        &DeliveryPolicy {
            retry_delays: Vec::new(),
            chunk_markers: true,
        },
        |identity, chunk_index, chunk| async move {
            send_recorded_message_with_components(
                db,
                api.client(),
                channel_id,
                &IdempotentChunk {
                    domain: identity.domain(),
                    logical_key: identity.logical_key().into(),
                    chunk_index,
                    content: chunk,
                },
                &[],
            )
            .await
        },
    )
    .await
}

async fn deliver_reply_text_with<E, F, Fut>(
    source_message_id: Id<MessageMarker>,
    kind: MessageReplyKind,
    text: &str,
    policy: &DeliveryPolicy,
    mut send: F,
) -> Result<usize, DeliveryFailure<E>>
where
    F: FnMut(MessageReplyIdentity, usize, String) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let identity = MessageReplyIdentity::new(source_message_id, kind);
    let chunks = if kind == MessageReplyKind::SavedRequest {
        split_exact_delivery_chunks(text, policy.chunk_markers)
    } else {
        split_delivery_chunks(text, policy.chunk_markers)
    };
    deliver_chunks_indexed(chunks, policy, |chunk_index, chunk| {
        send(identity.clone(), chunk_index, chunk)
    })
    .await
}

pub async fn send_reply_once(
    db: &std::path::Path,
    api: &DiscordHttp,
    channel_id: Id<ChannelMarker>,
    source_message_id: Id<MessageMarker>,
    kind: MessageReplyKind,
    text: &str,
    components: &[Component],
) -> Result<(), CompletionWorkerError> {
    send_reply_once_with(
        source_message_id,
        kind,
        text,
        components,
        |identity, chunk_index, content, components| async move {
            send_recorded_message_with_components(
                db,
                api.client(),
                channel_id,
                &IdempotentChunk {
                    domain: identity.domain(),
                    logical_key: identity.logical_key().into(),
                    chunk_index,
                    content,
                },
                &components,
            )
            .await
        },
    )
    .await
}

async fn send_reply_once_with<E, F, Fut>(
    source_message_id: Id<MessageMarker>,
    kind: MessageReplyKind,
    text: &str,
    components: &[Component],
    send: F,
) -> Result<(), E>
where
    F: FnOnce(MessageReplyIdentity, usize, String, Vec<Component>) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let identity = MessageReplyIdentity::new(source_message_id, kind);
    send(identity, 0, text.to_owned(), components.to_vec()).await
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use cdr_discord::components::busy_button_row;

    use super::*;

    #[tokio::test]
    async fn failed_chunk_retry_reuses_the_actual_delivery_identity_and_index() {
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let fail_first = Arc::new(AtomicBool::new(true));
        let observed = Arc::clone(&attempts);
        let injected_failure = Arc::clone(&fail_first);
        let policy = DeliveryPolicy {
            retry_delays: vec![Duration::ZERO],
            chunk_markers: true,
        };

        deliver_reply_text_with(
            Id::new(42),
            MessageReplyKind::ActionResult,
            "reply",
            &policy,
            move |identity, chunk_index, chunk| {
                let observed = Arc::clone(&observed);
                let injected_failure = Arc::clone(&injected_failure);
                async move {
                    observed
                        .lock()
                        .unwrap()
                        .push((identity, chunk_index, chunk));
                    if injected_failure.swap(false, Ordering::SeqCst) {
                        Err("injected first-attempt failure")
                    } else {
                        Ok(())
                    }
                }
            },
        )
        .await
        .unwrap();

        let attempts = attempts.lock().unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], attempts[1]);
        assert_eq!(attempts[0].1, 0);
    }

    #[tokio::test]
    async fn one_shot_delivery_forwards_components_with_the_source_identity() {
        let components = vec![busy_button_row("0123456789abcdef01234567", true).unwrap()];
        let expected = components.clone();

        send_reply_once_with(
            Id::new(42),
            MessageReplyKind::ActionResult,
            "Choose an action",
            &components,
            move |identity, chunk_index, content, components| async move {
                assert_eq!(identity.logical_key(), "inbound-message/42/action-result");
                assert_eq!(chunk_index, 0);
                assert_eq!(content, "Choose an action");
                assert_eq!(components, expected);
                Ok::<(), ()>(())
            },
        )
        .await
        .unwrap();
    }
}
