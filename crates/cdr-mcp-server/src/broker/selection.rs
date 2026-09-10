use cdr_remote_protocol::message::{BridgeResult, GatewayCommand, ProjectInfoOutput};
use chrono::Utc;
use uuid::Uuid;

use super::BridgeBroker;
use super::connection::require_connection;
use super::dispatch::{request_id, require_session_result};
use super::error::BrokerError;
use super::model::{
    BridgeIdentity, ProjectRegistration, ProjectSelection, SelectionTarget, SessionRoute,
};

impl BridgeBroker {
    pub async fn resume_project(
        &self,
        identity: &BridgeIdentity,
        project: &ProjectRegistration,
    ) -> Result<bool, BrokerError> {
        let _selection = self.selection.lock().await;
        let route = {
            let mut state = self.state.lock().await;
            require_connection(&state, identity)?;
            let now = Utc::now();
            state
                .dormant
                .retain(|_, value| value.resume_until > now && value.route.expires_at > now);
            let current_matches = state
                .projects
                .get(&project.project_scope)
                .is_some_and(|(owner, value)| owner == &identity.device_id && value == project);
            if !current_matches {
                return Ok(false);
            }
            let key = (identity.device_id.clone(), project.thread_id.clone());
            let Some(dormant) = state.dormant.remove(&key) else {
                return Ok(false);
            };
            if dormant.project_scope != project.project_scope
                || dormant.binding_id != project.binding_id
                || dormant.resume_until <= now
            {
                return Ok(false);
            }
            let generation = state
                .generations
                .get(&key)
                .copied()
                .unwrap_or(0)
                .max(dormant.route.generation)
                + 1;
            state.generations.insert(key, generation);
            let route = SessionRoute {
                session: dormant.route.session,
                subject: dormant.route.subject,
                device_id: identity.device_id.clone(),
                thread_id: project.thread_id.clone(),
                computer_session_id: Uuid::new_v4().simple().to_string(),
                generation,
                expires_at: project.expires_at,
                target: SelectionTarget::Project,
            };
            state.sessions.insert(route.session.clone(), route.clone());
            route
        };
        let command = GatewayCommand::ProjectSession {
            request_id: request_id(),
            thread_id: route.thread_id.clone(),
            deadline_at: cdr_core::deadline::default_request_deadline(),
            computer_session_id: route.computer_session_id.clone(),
            computer_session_generation: route.generation,
        };
        let result = self.dispatch_on_route(&route, command).await;
        let resumed = result.and_then(require_session_result).is_ok();
        if !resumed {
            self.remove_route_if_current(&route).await;
        }
        Ok(resumed)
    }

    pub async fn select_project(
        &self,
        session: &str,
        subject: &str,
        project_scope: &str,
    ) -> Result<ProjectSelection, BrokerError> {
        let _selection = self.selection.lock().await;
        let (route, project) = self
            .prepare_project_route(session, subject, project_scope)
            .await?;
        let command = GatewayCommand::ProjectSession {
            request_id: request_id(),
            thread_id: route.thread_id.clone(),
            deadline_at: cdr_core::deadline::default_request_deadline(),
            computer_session_id: route.computer_session_id.clone(),
            computer_session_generation: route.generation,
        };
        let activated = self.dispatch_on_route(&route, command).await;
        if !matches!(activated, Ok(BridgeResult::ProjectSessionResult { .. })) {
            self.remove_route_if_current(&route).await;
        }
        require_session_result(activated?)?;
        Ok(ProjectSelection {
            project_name: project.project_name,
            thread_id: route.thread_id,
            expires_at: route.expires_at,
        })
    }

    pub async fn project_info(
        &self,
        session: &str,
        subject: &str,
    ) -> Result<ProjectInfoOutput, BrokerError> {
        let route = self.active_route(session, subject).await?;
        let result = self
            .dispatch_on_route(
                &route,
                GatewayCommand::ProjectInfo {
                    request_id: request_id(),
                    thread_id: route.thread_id.clone(),
                    deadline_at: cdr_core::deadline::default_request_deadline(),
                    computer_session_id: Some(route.computer_session_id.clone()),
                },
            )
            .await?;
        match result {
            BridgeResult::ProjectInfoResult { output, .. } => Ok(output),
            BridgeResult::OperationError { message, .. } => {
                Err(BrokerError::RemoteOperation(message))
            }
            _ => Err(BrokerError::WrongResultType),
        }
    }

    async fn prepare_project_route(
        &self,
        session: &str,
        subject: &str,
        scope: &str,
    ) -> Result<(SessionRoute, ProjectRegistration), BrokerError> {
        let mut state = self.state.lock().await;
        let now = Utc::now();
        let (device_id, project) = state
            .projects
            .get(scope)
            .filter(|(_, value)| value.expires_at > now)
            .cloned()
            .ok_or(BrokerError::ProjectUnavailable)?;
        if !state.devices.contains_key(&device_id) {
            return Err(BrokerError::BridgeUnavailable);
        }
        if let Some(existing) = state.sessions.get(session)
            && existing.subject != subject
        {
            return Err(BrokerError::PrincipalMismatch);
        }
        let key = (device_id.clone(), project.thread_id.clone());
        let generation = state.generations.get(&key).copied().unwrap_or(0) + 1;
        state.generations.insert(key, generation);
        let route = SessionRoute {
            session: session.to_owned(),
            subject: subject.to_owned(),
            device_id,
            thread_id: project.thread_id.clone(),
            computer_session_id: Uuid::new_v4().simple().to_string(),
            generation,
            expires_at: project.expires_at,
            target: SelectionTarget::Project,
        };
        state.sessions.insert(session.to_owned(), route.clone());
        Ok((route, project))
    }

    pub(super) async fn active_route(
        &self,
        session: &str,
        subject: &str,
    ) -> Result<SessionRoute, BrokerError> {
        let mut state = self.state.lock().await;
        let now = Utc::now();
        state.sessions.retain(|_, route| route.expires_at > now);
        let route = state
            .sessions
            .get(session)
            .cloned()
            .ok_or(BrokerError::ActiveSelectionMissing)?;
        if route.subject != subject {
            return Err(BrokerError::PrincipalMismatch);
        }
        if !state.devices.contains_key(&route.device_id) {
            return Err(BrokerError::BridgeUnavailable);
        }
        Ok(route)
    }
}
