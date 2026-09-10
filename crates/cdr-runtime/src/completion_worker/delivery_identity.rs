use cdr_discord::delivery::{DeliveryFailure, DeliveryPolicy, deliver_text_indexed};
use sha2::{Digest, Sha256};

const COMPLETION_NONCE_DOMAIN: &str = "completion/v1";
const GOAL_PROGRESS_NONCE_DOMAIN: &str = "completion/goal-progress/v1";
const COMMENTARY_NONCE_DOMAIN: &str = "completion/commentary/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CompletionDeliveryIdentity {
    domain: &'static str,
    logical_key: String,
}

impl CompletionDeliveryIdentity {
    #[must_use]
    pub(super) fn outbox(delivery_id: &str) -> Self {
        Self {
            domain: COMPLETION_NONCE_DOMAIN,
            logical_key: delivery_id.to_owned(),
        }
    }

    #[must_use]
    pub(super) fn goal_progress(thread_id: &str, turn_id: &str) -> Self {
        Self {
            domain: GOAL_PROGRESS_NONCE_DOMAIN,
            logical_key: length_prefixed_key([thread_id, turn_id]),
        }
    }

    #[must_use]
    pub(super) fn commentary(thread_id: &str, turn_id: &str, text: &str) -> Self {
        let mut digest = Sha256::new();
        digest.update(text.trim().as_bytes());
        digest.update([0]);
        let text_digest = hex::encode(digest.finalize());
        Self {
            domain: COMMENTARY_NONCE_DOMAIN,
            logical_key: length_prefixed_key([thread_id, turn_id, &text_digest]),
        }
    }

    #[must_use]
    pub(super) const fn domain(&self) -> &'static str {
        self.domain
    }

    #[must_use]
    pub(super) fn logical_key(&self) -> &str {
        &self.logical_key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IdempotentChunk {
    pub domain: &'static str,
    pub logical_key: String,
    pub chunk_index: usize,
    pub content: String,
}

pub(super) async fn deliver_idempotent_chunks<E, F, Fut>(
    text: &str,
    policy: &DeliveryPolicy,
    identity: &CompletionDeliveryIdentity,
    mut send: F,
) -> Result<usize, DeliveryFailure<E>>
where
    F: FnMut(IdempotentChunk) -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
{
    deliver_text_indexed(text, policy, |chunk_index, chunk| {
        send(IdempotentChunk {
            domain: identity.domain(),
            logical_key: identity.logical_key().to_owned(),
            chunk_index,
            content: chunk,
        })
    })
    .await
}

fn length_prefixed_key<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut key = String::new();
    for part in parts {
        key.push_str(&part.len().to_string());
        key.push(':');
        key.push_str(part);
        key.push(';');
    }
    key
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn completion_chunks_carry_delivery_id_and_reuse_it_on_retry() {
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&attempts);
        let mut fail_once = true;
        let identity = CompletionDeliveryIdentity::outbox("delivery-42");

        let sent = deliver_idempotent_chunks(
            &"x".repeat(2_100),
            &DeliveryPolicy {
                retry_delays: vec![Duration::ZERO],
                chunk_markers: true,
            },
            &identity,
            move |chunk| {
                recorded.lock().unwrap().push(chunk);
                let fail = fail_once;
                fail_once = false;
                async move { if fail { Err("temporary") } else { Ok(()) } }
            },
        )
        .await
        .unwrap();

        assert_eq!(sent, 2);
        let attempts = attempts.lock().unwrap();
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[0], attempts[1]);
        assert_eq!(attempts[0].domain, COMPLETION_NONCE_DOMAIN);
        assert_eq!(attempts[0].logical_key, "delivery-42");
        assert_eq!(attempts[0].chunk_index, 0);
        assert_eq!(attempts[2].domain, COMPLETION_NONCE_DOMAIN);
        assert_eq!(attempts[2].logical_key, "delivery-42");
        assert_eq!(attempts[2].chunk_index, 1);
        assert_ne!(attempts[1].content, attempts[2].content);
    }

    #[test]
    fn progress_and_commentary_identities_are_stable_and_domain_separated() {
        let progress = CompletionDeliveryIdentity::goal_progress("thread-1", "turn-1");
        assert_eq!(
            progress,
            CompletionDeliveryIdentity::goal_progress("thread-1", "turn-1")
        );
        assert_ne!(
            progress,
            CompletionDeliveryIdentity::goal_progress("thread-1", "turn-2")
        );
        assert_eq!(progress.domain(), GOAL_PROGRESS_NONCE_DOMAIN);

        let commentary =
            CompletionDeliveryIdentity::commentary("thread-1", "turn-1", "  same update  ");
        assert_eq!(
            commentary,
            CompletionDeliveryIdentity::commentary("thread-1", "turn-1", "same update")
        );
        assert_ne!(
            commentary,
            CompletionDeliveryIdentity::commentary("thread-1", "turn-1", "different update")
        );
        assert_ne!(
            commentary,
            CompletionDeliveryIdentity::commentary("thread-2", "turn-1", "same update")
        );
        assert_eq!(commentary.domain(), COMMENTARY_NONCE_DOMAIN);
        assert_ne!(progress.domain(), commentary.domain());
    }
}
