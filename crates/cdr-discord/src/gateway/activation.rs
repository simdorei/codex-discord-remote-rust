#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use thiserror::Error;
use tokio::sync::{Notify, mpsc, watch};
use twilight_gateway::Config;
use twilight_http::Client;

use super::{
    GatewayIngressConfig, GatewayIngressPublishOutcomes, GatewayRuntime, GatewayStartError,
    gateway_intents, ingress::GatewayIngress, shard::run_shard, shutdown::GatewayTask,
};

const PAUSED: u8 = 0;
const ACTIVATED: u8 = 1;
const STOPPED: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum GatewayActivationError {
    #[error("Discord gateway ingress receivers must be taken before typed activation")]
    IngressReceiversNotTaken,
    #[error("Discord gateway typed consumers have already been activated")]
    AlreadyActivated,
    #[error("Discord gateway activation was stopped by shutdown")]
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(super) enum ShardActivationError {
    #[error("gateway has already been activated")]
    AlreadyActivated,
    #[error("gateway activation was stopped")]
    Stopped,
}

impl From<ShardActivationError> for GatewayActivationError {
    fn from(error: ShardActivationError) -> Self {
        match error {
            ShardActivationError::AlreadyActivated => Self::AlreadyActivated,
            ShardActivationError::Stopped => Self::Stopped,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ActivationWaitOutcome {
    Activated,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ActivationStopOutcome {
    StoppedBeforeActivation,
    StoppedAfterActivation,
    AlreadyStopped,
}

struct ActivationInner {
    state: AtomicU8,
    changed: Notify,
    #[cfg(test)]
    notifications: AtomicUsize,
}

#[derive(Clone)]
pub(super) struct ShardActivation {
    inner: Arc<ActivationInner>,
}

impl ShardActivation {
    pub(super) fn paused() -> Self {
        Self::with_state(PAUSED)
    }

    fn with_state(state: u8) -> Self {
        Self {
            inner: Arc::new(ActivationInner {
                state: AtomicU8::new(state),
                changed: Notify::new(),
                #[cfg(test)]
                notifications: AtomicUsize::new(0),
            }),
        }
    }

    pub(super) fn activate(&self) -> Result<(), ShardActivationError> {
        match self.inner.state.compare_exchange(
            PAUSED,
            ACTIVATED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                self.notify_waiters();
                Ok(())
            }
            Err(ACTIVATED) => Err(ShardActivationError::AlreadyActivated),
            Err(STOPPED) => Err(ShardActivationError::Stopped),
            Err(_) => unreachable!("activation state is always valid"),
        }
    }

    /// Activation and stop linearize at their atomic state transition. If stop changes a paused
    /// gate first, every later activation fails and no shard entry can become active.
    pub(super) fn stop(&self) -> ActivationStopOutcome {
        let previous = self.inner.state.swap(STOPPED, Ordering::AcqRel);
        if previous != STOPPED {
            self.notify_waiters();
        }
        match previous {
            PAUSED => ActivationStopOutcome::StoppedBeforeActivation,
            ACTIVATED => ActivationStopOutcome::StoppedAfterActivation,
            STOPPED => ActivationStopOutcome::AlreadyStopped,
            _ => unreachable!("activation state is always valid"),
        }
    }

    pub(super) async fn wait(&self) -> ActivationWaitOutcome {
        self.wait_with_probe(|| {}).await
    }

    #[cfg(test)]
    pub(super) async fn wait_with_arming_probe<F>(&self, probe: F) -> ActivationWaitOutcome
    where
        F: FnOnce(),
    {
        self.wait_with_probe(probe).await
    }

    async fn wait_with_probe<F>(&self, probe: F) -> ActivationWaitOutcome
    where
        F: FnOnce(),
    {
        let mut probe = Some(probe);
        loop {
            let notified = self.inner.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(probe) = probe.take() {
                probe();
            }
            match self.inner.state.load(Ordering::Acquire) {
                ACTIVATED => return ActivationWaitOutcome::Activated,
                STOPPED => return ActivationWaitOutcome::Stopped,
                PAUSED => notified.await,
                _ => unreachable!("activation state is always valid"),
            }
        }
    }

    fn notify_waiters(&self) {
        #[cfg(test)]
        self.inner.notifications.fetch_add(1, Ordering::Relaxed);
        self.inner.changed.notify_waiters();
    }

    #[cfg(test)]
    pub(super) fn is_paused(&self) -> bool {
        self.inner.state.load(Ordering::Acquire) == PAUSED
    }

    #[cfg(test)]
    pub(super) fn is_activated(&self) -> bool {
        self.inner.state.load(Ordering::Acquire) == ACTIVATED
    }

    #[cfg(test)]
    pub(super) fn notification_count(&self) -> usize {
        self.inner.notifications.load(Ordering::Relaxed)
    }
}

impl GatewayRuntime {
    /// Creates gateway shard tasks that remain paused until typed consumers are ready.
    ///
    /// Call [`Self::take_ingress_receivers`], spawn consumers for every receiver lane, and then call
    /// [`Self::activate_typed_consumers`]. Shutdown or drop before activation ends every waiter.
    pub async fn start_paused(
        token: String,
        message_content: bool,
    ) -> Result<Self, GatewayStartError> {
        Self::start_with_activation(token, message_content, ShardActivation::paused()).await
    }

    /// Activates shard polling after ownership of all typed receiver lanes has transferred.
    pub fn activate_typed_consumers(&self) -> Result<(), GatewayActivationError> {
        if self.ingress_receivers.is_some() {
            return Err(GatewayActivationError::IngressReceiversNotTaken);
        }
        self.activation.activate().map_err(Into::into)
    }

    pub(super) async fn start_with_activation(
        token: String,
        message_content: bool,
        activation: ShardActivation,
    ) -> Result<Self, GatewayStartError> {
        let (ingress, ingress_receivers) = GatewayIngress::new(GatewayIngressConfig::default())?;
        let ingress_publish_outcomes = GatewayIngressPublishOutcomes::default();
        let http = Arc::new(Client::new(token.clone()));
        let config = Config::new(token, gateway_intents(message_content));
        let shards =
            twilight_gateway::create_recommended(&http, config, |_, builder| builder.build())
                .await?;
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (shard_exit_sender, shard_exits) = mpsc::channel(1);
        let tasks = shards
            .map(|shard| {
                let shard_id = shard.id().number();
                GatewayTask::new(
                    shard_id,
                    tokio::spawn(run_shard(
                        shard,
                        ingress.clone(),
                        ingress_publish_outcomes.clone(),
                        shutdown_rx.clone(),
                        activation.clone(),
                        shard_exit_sender.clone(),
                    )),
                )
            })
            .collect();
        drop(shard_exit_sender);
        Ok(Self {
            ingress,
            ingress_receivers: Some(ingress_receivers),
            ingress_publish_outcomes,
            activation,
            shutdown,
            tasks,
            shard_exits,
            http,
        })
    }
}
