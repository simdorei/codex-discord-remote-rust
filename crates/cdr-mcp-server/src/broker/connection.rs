use cdr_remote_protocol::message::BridgeResult;
use chrono::Utc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::BridgeBroker;
use super::error::BrokerError;
use super::model::{BridgeAttachment, BridgeIdentity, DeviceSummary, ProjectRegistration};
use super::state::{BrokerState, DeviceConnection};

const COMMAND_QUEUE_CAPACITY: usize = 32;

impl BridgeBroker {
    pub async fn attach(&self, device_id: &str) -> BridgeAttachment {
        let identity = BridgeIdentity {
            device_id: device_id.to_owned(),
            connection_id: Uuid::new_v4(),
        };
        let (commands, receiver) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let disconnected = CancellationToken::new();
        let mut state = self.state.lock().await;
        if let Some(previous_id) = state
            .devices
            .get(device_id)
            .map(|previous| previous.connection_id)
        {
            state.disconnect(device_id, previous_id);
        }
        state.devices.insert(
            device_id.to_owned(),
            DeviceConnection {
                connection_id: identity.connection_id,
                commands,
                disconnected: disconnected.clone(),
            },
        );
        BridgeAttachment::new(identity, receiver, disconnected)
    }

    pub async fn detach(&self, identity: &BridgeIdentity) {
        self.state
            .lock()
            .await
            .disconnect(&identity.device_id, identity.connection_id);
    }

    pub async fn list_devices(&self) -> Vec<DeviceSummary> {
        let state = self.state.lock().await;
        let mut devices = state
            .devices
            .keys()
            .map(|device_id| DeviceSummary {
                device_id: device_id.clone(),
                online: true,
            })
            .collect::<Vec<_>>();
        devices.sort_by_key(|value| value.device_id.to_ascii_lowercase());
        devices
    }

    pub async fn upsert(
        &self,
        identity: &BridgeIdentity,
        project: ProjectRegistration,
    ) -> Result<(), BrokerError> {
        let mut state = self.state.lock().await;
        require_connection(&state, identity)?;
        let now = Utc::now();
        state
            .projects
            .retain(|_, (_, value)| value.expires_at > now);
        state.sessions.retain(|_, route| route.expires_at > now);
        state
            .dormant
            .retain(|_, value| value.resume_until > now && value.route.expires_at > now);
        if project.expires_at <= now {
            return Err(BrokerError::ProjectUnavailable);
        }
        if let Some((owner, _)) = state.projects.get(&project.project_scope)
            && owner != &identity.device_id
        {
            return Err(BrokerError::ProjectOwnedByAnotherDevice);
        }
        if state.dormant.values().any(|value| {
            value.project_scope == project.project_scope
                && value.route.device_id != identity.device_id
        }) {
            return Err(BrokerError::ProjectOwnedByAnotherDevice);
        }
        let existing = state.projects.get(&project.project_scope).cloned();
        if let Some((owner, current)) = existing
            && owner == identity.device_id
            && current.binding_id == project.binding_id
            && current.thread_id == project.thread_id
            && current.project_name == project.project_name
        {
            state.projects.insert(
                project.project_scope.clone(),
                (identity.device_id.clone(), project.clone()),
            );
            for route in state.sessions.values_mut().filter(|route| {
                route.device_id == identity.device_id && route.thread_id == project.thread_id
            }) {
                route.expires_at = route.expires_at.max(project.expires_at);
            }
            if let Some(dormant) = state
                .dormant
                .get_mut(&(identity.device_id.clone(), project.thread_id.clone()))
            {
                dormant.route.expires_at = dormant.route.expires_at.max(project.expires_at);
            }
            return Ok(());
        }
        state.projects.retain(|scope, (owner, value)| {
            owner != &identity.device_id
                || value.thread_id != project.thread_id
                || scope == &project.project_scope
        });
        state.sessions.retain(|_, route| {
            route.device_id != identity.device_id || route.thread_id != project.thread_id
        });
        let dormant_key = (identity.device_id.clone(), project.thread_id.clone());
        if state.dormant.get(&dormant_key).is_some_and(|dormant| {
            dormant.project_scope != project.project_scope
                || dormant.binding_id != project.binding_id
        }) {
            state.dormant.remove(&dormant_key);
        }
        state.projects.insert(
            project.project_scope.clone(),
            (identity.device_id.clone(), project),
        );
        Ok(())
    }

    pub async fn complete(&self, identity: &BridgeIdentity, result: BridgeResult) {
        let request_id = result_request_id(&result).to_owned();
        let pending = {
            let mut state = self.state.lock().await;
            if require_connection(&state, identity).is_err() {
                return;
            }
            let matches = state.pending.get(&request_id).is_some_and(|pending| {
                pending.device_id == identity.device_id
                    && pending.connection_id == identity.connection_id
            });
            matches.then(|| state.pending.remove(&request_id)).flatten()
        };
        if let Some(pending) = pending {
            let _ = pending.response.send(result);
        }
    }
}

pub(super) fn require_connection(
    state: &BrokerState,
    identity: &BridgeIdentity,
) -> Result<(), BrokerError> {
    state
        .devices
        .get(&identity.device_id)
        .filter(|value| value.connection_id == identity.connection_id)
        .map(|_| ())
        .ok_or(BrokerError::BridgeUnavailable)
}

fn result_request_id(result: &BridgeResult) -> &str {
    match result {
        BridgeResult::ProjectInfoResult { request_id, .. }
        | BridgeResult::ListFilesResult { request_id, .. }
        | BridgeResult::ReadFileResult { request_id, .. }
        | BridgeResult::WriteFileResult { request_id, .. }
        | BridgeResult::ProjectOperationResult { request_id, .. }
        | BridgeResult::ProjectSessionResult { request_id }
        | BridgeResult::OperationError { request_id, .. } => request_id,
    }
}
