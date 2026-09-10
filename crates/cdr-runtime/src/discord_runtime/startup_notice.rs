use std::future::Future;

pub(super) const STARTUP_NOTICE_DOMAIN: &str = "runtime/startup-notice/v1";

/// Per-process startup delivery state.
///
/// A random logical key is created once by the typed Ready worker. Reconnects reuse
/// that key, while a later process run truthfully represents a new startup.
pub(super) struct StartupNoticeState {
    logical_key: String,
    delivered: bool,
}

impl StartupNoticeState {
    pub(super) fn new(logical_key: String) -> Self {
        Self {
            logical_key,
            delivered: false,
        }
    }

    pub(super) fn logical_key(&self) -> &str {
        &self.logical_key
    }

    pub(super) async fn try_send<F, Fut, E>(&mut self, send: F) -> Result<bool, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), E>>,
    {
        if self.delivered {
            return Ok(false);
        }
        send().await?;
        self.delivered = true;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cdr_discord::idempotent_message::message_nonce;
    use twilight_model::id::Id;

    use super::{STARTUP_NOTICE_DOMAIN, StartupNoticeState};

    #[test]
    fn reconnect_reuses_boot_nonce_while_a_new_runtime_is_distinct() {
        let current = StartupNoticeState::new("boot-a".into());
        let restarted = StartupNoticeState::new("boot-b".into());
        let channel = Id::new(91);
        let first = message_nonce(STARTUP_NOTICE_DOMAIN, channel, current.logical_key(), 0);

        assert_eq!(
            first,
            message_nonce(STARTUP_NOTICE_DOMAIN, channel, current.logical_key(), 0)
        );
        assert_ne!(
            first,
            message_nonce(STARTUP_NOTICE_DOMAIN, channel, restarted.logical_key(), 0)
        );
    }

    #[tokio::test]
    async fn successful_notice_is_not_sent_again_on_reconnect() {
        let calls = AtomicUsize::new(0);
        let mut state = StartupNoticeState::new("boot-a".into());

        assert!(
            state
                .try_send(|| async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, &'static str>(())
                })
                .await
                .unwrap()
        );
        assert!(
            !state
                .try_send(|| async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, &'static str>(())
                })
                .await
                .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_notice_remains_retryable_on_a_later_ready() {
        let calls = AtomicUsize::new(0);
        let mut state = StartupNoticeState::new("boot-a".into());

        let first = state
            .try_send(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>("Discord unavailable")
            })
            .await;
        assert_eq!(first, Err("Discord unavailable"));
        assert!(
            state
                .try_send(|| async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, &'static str>(())
                })
                .await
                .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
