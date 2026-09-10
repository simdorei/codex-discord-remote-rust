use cdr_remote_protocol::message::GatewayCommand;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeIdentity {
    pub(super) device_id: String,
    pub(super) connection_id: Uuid,
}

impl BridgeIdentity {
    #[must_use]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }
}

pub struct BridgeAttachment {
    identity: BridgeIdentity,
    pub commands: mpsc::Receiver<GatewayCommand>,
    pub disconnected: CancellationToken,
}

impl BridgeAttachment {
    #[must_use]
    pub fn identity(&self) -> &BridgeIdentity {
        &self.identity
    }

    pub(super) fn new(
        identity: BridgeIdentity,
        commands: mpsc::Receiver<GatewayCommand>,
        disconnected: CancellationToken,
    ) -> Self {
        Self {
            identity,
            commands,
            disconnected,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRegistration {
    pub project_scope: String,
    pub binding_id: String,
    pub thread_id: String,
    pub project_name: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectSelection {
    pub project_name: String,
    pub thread_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceSummary {
    pub device_id: String,
    pub online: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceSelection {
    pub device_id: String,
    pub working_directory: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub(super) enum SelectionTarget {
    Project,
    Device { working_directory: String },
}

#[derive(Clone)]
pub(super) struct SessionRoute {
    pub session: String,
    pub subject: String,
    pub device_id: String,
    pub thread_id: String,
    pub computer_session_id: String,
    pub generation: u64,
    pub expires_at: DateTime<Utc>,
    pub target: SelectionTarget,
}

#[derive(Clone)]
pub(super) struct DormantRoute {
    pub route: SessionRoute,
    pub project_scope: String,
    pub binding_id: String,
    pub resume_until: DateTime<Utc>,
}
