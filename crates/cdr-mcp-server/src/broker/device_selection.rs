use cdr_remote_protocol::message::GatewayCommand;
use chrono::{TimeDelta, Utc};
use uuid::Uuid;

use super::BridgeBroker;
use super::dispatch::{request_id, require_session_result};
use super::error::BrokerError;
use super::model::{DeviceSelection, SelectionTarget, SessionRoute};

const DEVICE_THREAD_ID: &str = "codex-device-control";
const DEVICE_ROUTE_TTL_SECONDS: i64 = 1_800;

impl BridgeBroker {
    pub async fn select_device(
        &self,
        session: &str,
        subject: &str,
        device_id: &str,
        working_directory: &str,
    ) -> Result<DeviceSelection, BrokerError> {
        validate_directory(working_directory)?;
        let _selection = self.selection.lock().await;
        let route = self
            .prepare_device_route(session, subject, device_id, working_directory)
            .await?;
        self.activate_device_route(&route).await?;
        Ok(device_selection(&route))
    }

    pub async fn set_working_directory(
        &self,
        session: &str,
        subject: &str,
        working_directory: &str,
    ) -> Result<DeviceSelection, BrokerError> {
        validate_directory(working_directory)?;
        let _selection = self.selection.lock().await;
        let current = self.active_route(session, subject).await?;
        if !matches!(current.target, SelectionTarget::Device { .. }) {
            return Err(BrokerError::ActiveSelectionMissing);
        }
        let route = self
            .prepare_device_route(session, subject, &current.device_id, working_directory)
            .await?;
        self.activate_device_route(&route).await?;
        Ok(device_selection(&route))
    }

    pub async fn device_info(
        &self,
        session: &str,
        subject: &str,
    ) -> Result<DeviceSelection, BrokerError> {
        let route = self.active_route(session, subject).await?;
        if !matches!(route.target, SelectionTarget::Device { .. }) {
            return Err(BrokerError::ActiveSelectionMissing);
        }
        Ok(device_selection(&route))
    }

    async fn prepare_device_route(
        &self,
        session: &str,
        subject: &str,
        device_id: &str,
        working_directory: &str,
    ) -> Result<SessionRoute, BrokerError> {
        let mut state = self.state.lock().await;
        if !state.devices.contains_key(device_id) {
            return Err(BrokerError::BridgeUnavailable);
        }
        if let Some(existing) = state.sessions.get(session)
            && existing.subject != subject
        {
            return Err(BrokerError::PrincipalMismatch);
        }
        let key = (device_id.to_owned(), DEVICE_THREAD_ID.to_owned());
        let generation = state.generations.get(&key).copied().unwrap_or(0) + 1;
        state.generations.insert(key, generation);
        let route = SessionRoute {
            session: session.to_owned(),
            subject: subject.to_owned(),
            device_id: device_id.to_owned(),
            thread_id: DEVICE_THREAD_ID.to_owned(),
            computer_session_id: Uuid::new_v4().simple().to_string(),
            generation,
            expires_at: Utc::now() + TimeDelta::seconds(DEVICE_ROUTE_TTL_SECONDS),
            target: SelectionTarget::Device {
                working_directory: working_directory.to_owned(),
            },
        };
        state.sessions.insert(session.to_owned(), route.clone());
        Ok(route)
    }

    async fn activate_device_route(&self, route: &SessionRoute) -> Result<(), BrokerError> {
        let SelectionTarget::Device { working_directory } = &route.target else {
            return Err(BrokerError::ActiveSelectionMissing);
        };
        let command = GatewayCommand::DeviceSession {
            request_id: request_id(),
            thread_id: route.thread_id.clone(),
            deadline_at: cdr_core::deadline::default_request_deadline(),
            computer_session_id: route.computer_session_id.clone(),
            computer_session_generation: route.generation,
            working_directory: working_directory.clone(),
            expires_at: route.expires_at,
        };
        let result = self.dispatch_on_route(route, command).await;
        if result.is_err() {
            self.remove_route_if_current(route).await;
        }
        require_session_result(result?)
    }
}

fn device_selection(route: &SessionRoute) -> DeviceSelection {
    let SelectionTarget::Device { working_directory } = &route.target else {
        unreachable!("device selection requires a device route")
    };
    DeviceSelection {
        device_id: route.device_id.clone(),
        working_directory: working_directory.clone(),
        expires_at: route.expires_at,
    }
}

fn validate_directory(value: &str) -> Result<(), BrokerError> {
    if value.is_empty() || value.len() > 1_000 {
        Err(BrokerError::InvalidRequest(
            "working_directory must contain 1 to 1000 characters".into(),
        ))
    } else {
        Ok(())
    }
}
