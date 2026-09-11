use std::sync::Arc;

use cdr_remote_protocol::message::BridgeResult;

use super::LocalProjectDispatcher;
use super::error::session_error;
use super::state::{ActiveProject, SessionActivation};
use crate::computer::{ComputerAccessMode, SessionComputer};
use crate::terminal::{TerminalExecutionEngine, TerminalWindowManager};

impl LocalProjectDispatcher {
    pub async fn begin_connection(&self, generation: u64) -> Result<(), String> {
        let projects = {
            let mut state = self.state.write().await;
            if state
                .connection_generation
                .is_some_and(|current| generation <= current)
            {
                return Err("The local bridge connection generation did not advance.".into());
            }
            state.connection_generation = Some(generation);
            state.projects.values().cloned().collect::<Vec<_>>()
        };
        close_project_sessions(projects).await
    }

    pub async fn retire_sessions(&self) -> Result<(), String> {
        let projects = {
            let mut state = self.state.write().await;
            state.connection_generation = None;
            state.projects.values().cloned().collect::<Vec<_>>()
        };
        close_project_sessions(projects).await
    }

    pub(super) async fn activate(
        &self,
        project: &ActiveProject,
        request_id: &str,
        session_id: &str,
        session_generation: u64,
        computer_mode: ComputerAccessMode,
        connection_generation: Option<u64>,
    ) -> BridgeResult {
        let current_connection = self.state.read().await.connection_generation;
        if connection_generation.is_some() && current_connection != connection_generation {
            return session_error(
                request_id,
                "The command belongs to a stale local bridge connection.",
            );
        }
        let mut current = project.session.lock().await;
        if let Some(active) = current.as_ref() {
            if active.connection_generation != connection_generation {
                return session_error(
                    request_id,
                    "The project session belongs to a stale bridge connection.",
                );
            }
            if session_generation < active.session_generation {
                return session_error(
                    request_id,
                    "The project session command is older than the active session.",
                );
            }
            if session_generation == active.session_generation && session_id != active.session_id {
                return session_error(
                    request_id,
                    "The project session generation conflicts with another session.",
                );
            }
            if session_generation == active.session_generation
                && session_id == active.session_id
                && computer_mode != active.computer_mode
            {
                return session_error(
                    request_id,
                    "The project session mode conflicts with the active session.",
                );
            }
            if session_generation == active.session_generation && session_id == active.session_id {
                return activation_result(request_id);
            }
        }
        let terminals = match TerminalExecutionEngine::new(project.access.root(), session_id) {
            Ok(value) => value,
            Err(error) => return session_error(request_id, &error.to_string()),
        };
        let terminal_windows = match TerminalWindowManager::new(project.access.root()) {
            Ok(value) => value,
            Err(error) => return session_error(request_id, &error.to_string()),
        };
        let computer = SessionComputer::new(computer_mode);
        let previous = current.replace(Arc::new(SessionActivation {
            connection_generation,
            session_generation,
            session_id: session_id.to_owned(),
            computer_mode,
            computer,
            terminals,
            terminal_windows,
        }));
        drop(current);
        if let Some(previous) = previous
            && let Err(error) = close_session(&previous).await
        {
            return session_error(request_id, &error);
        }
        activation_result(request_id)
    }

    pub(super) async fn require_session(
        &self,
        project: &ActiveProject,
        session_id: Option<&str>,
        connection_generation: Option<u64>,
    ) -> Result<Arc<SessionActivation>, String> {
        if connection_generation.is_some()
            && self.state.read().await.connection_generation != connection_generation
        {
            return Err("The command belongs to a stale local bridge connection.".into());
        }
        let active = project.session.lock().await.clone();
        active
            .filter(|active| {
                Some(active.session_id.as_str()) == session_id
                    && active.connection_generation == connection_generation
            })
            .ok_or_else(|| "The ChatGPT session is stale or was not acknowledged locally.".into())
    }
}

async fn close_project_sessions(projects: Vec<Arc<ActiveProject>>) -> Result<(), String> {
    let mut sessions = Vec::new();
    for project in projects {
        if let Some(session) = project.session.lock().await.take() {
            sessions.push(session);
        }
    }
    for session in sessions {
        close_session(&session).await?;
    }
    Ok(())
}

pub(super) async fn close_session(session: &Arc<SessionActivation>) -> Result<(), String> {
    let computer = Arc::clone(session);
    let computer_result = tokio::task::spawn_blocking(move || {
        computer
            .computer
            .execute(&cdr_remote_protocol::request::ComputerRequest::ComputerStop)
    })
    .await
    .map_err(|error| format!("computer stop worker failed: {error}"))
    .and_then(|result| result.map(|_| ()).map_err(|error| error.to_string()));
    let window_result = session
        .terminal_windows
        .close_all()
        .map_err(|error| error.to_string());
    let terminal_result = session
        .terminals
        .close()
        .await
        .map_err(|error| error.to_string());
    computer_result.and(window_result).and(terminal_result)
}

fn activation_result(request_id: &str) -> BridgeResult {
    BridgeResult::ProjectSessionResult {
        request_id: request_id.to_owned(),
    }
}
