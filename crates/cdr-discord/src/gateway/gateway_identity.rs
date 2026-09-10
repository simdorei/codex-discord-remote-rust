use std::sync::{Arc, OnceLock};

use thiserror::Error;
use tokio::sync::broadcast;
use twilight_model::{
    gateway::event::Event,
    id::{
        Id,
        marker::{ApplicationMarker, UserMarker},
    },
};

const IDENTITY_NOTIFICATION_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayIdentity {
    pub user_id: Id<UserMarker>,
    pub application_id: Id<ApplicationMarker>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("Discord gateway READY identity changed from {established:?} to {observed:?}")]
pub struct GatewayIdentityConflict {
    pub established: GatewayIdentity,
    pub observed: GatewayIdentity,
}

#[derive(Debug, Default)]
struct GatewayIdentityState {
    identity: OnceLock<GatewayIdentity>,
    conflict: OnceLock<GatewayIdentityConflict>,
}

#[derive(Clone, Debug)]
pub(super) struct GatewayIdentityTracker {
    state: Arc<GatewayIdentityState>,
    identity_notifications: broadcast::Sender<()>,
    conflict_notifications: broadcast::Sender<()>,
}

impl GatewayIdentityTracker {
    pub(super) fn new() -> Self {
        let (identity_notifications, _) = broadcast::channel(IDENTITY_NOTIFICATION_CAPACITY);
        let (conflict_notifications, _) = broadcast::channel(IDENTITY_NOTIFICATION_CAPACITY);
        Self {
            state: Arc::new(GatewayIdentityState::default()),
            identity_notifications,
            conflict_notifications,
        }
    }

    fn observe(&self, observed: GatewayIdentity) {
        if self.state.identity.set(observed).is_ok() {
            let _ = self.identity_notifications.send(());
        } else {
            let established = *self
                .state
                .identity
                .get()
                .expect("failed OnceLock set leaves an established identity");
            if established != observed
                && self
                    .state
                    .conflict
                    .set(GatewayIdentityConflict {
                        established,
                        observed,
                    })
                    .is_ok()
            {
                let _ = self.conflict_notifications.send(());
            }
        }
    }

    pub(super) fn subscribe_identity(&self) -> GatewayIdentityReceiver {
        GatewayIdentityReceiver {
            state: Arc::clone(&self.state),
            notifications: self.identity_notifications.subscribe(),
        }
    }

    pub(super) fn subscribe_conflict(&self) -> GatewayIdentityConflictReceiver {
        GatewayIdentityConflictReceiver {
            state: Arc::clone(&self.state),
            notifications: self.conflict_notifications.subscribe(),
        }
    }
}

#[derive(Debug)]
pub struct GatewayIdentityReceiver {
    state: Arc<GatewayIdentityState>,
    notifications: broadcast::Receiver<()>,
}

impl GatewayIdentityReceiver {
    /// Read durable state first; notification delivery is only a bounded hint.
    #[must_use]
    pub fn snapshot(&self) -> Option<GatewayIdentity> {
        self.state.identity.get().copied()
    }

    pub async fn changed(
        &mut self,
    ) -> Result<Option<GatewayIdentity>, broadcast::error::RecvError> {
        self.notifications.recv().await?;
        Ok(self.snapshot())
    }

    pub fn try_changed(
        &mut self,
    ) -> Result<Option<GatewayIdentity>, broadcast::error::TryRecvError> {
        self.notifications.try_recv()?;
        Ok(self.snapshot())
    }
}

#[derive(Debug)]
pub struct GatewayIdentityConflictReceiver {
    state: Arc<GatewayIdentityState>,
    notifications: broadcast::Receiver<()>,
}

impl GatewayIdentityConflictReceiver {
    /// Read durable state first; any retained conflict is fatal.
    #[must_use]
    pub fn snapshot(&self) -> Option<GatewayIdentityConflict> {
        self.state.conflict.get().copied()
    }

    pub async fn changed(
        &mut self,
    ) -> Result<Option<GatewayIdentityConflict>, broadcast::error::RecvError> {
        self.notifications.recv().await?;
        Ok(self.snapshot())
    }

    pub fn try_changed(
        &mut self,
    ) -> Result<Option<GatewayIdentityConflict>, broadcast::error::TryRecvError> {
        self.notifications.try_recv()?;
        Ok(self.snapshot())
    }
}

pub(super) fn publish_gateway_event_identity(event: &Event, identity: &GatewayIdentityTracker) {
    if let Event::Ready(ready) = event {
        identity.observe(GatewayIdentity {
            user_id: ready.user.id,
            application_id: ready.application.id,
        });
    }
}
