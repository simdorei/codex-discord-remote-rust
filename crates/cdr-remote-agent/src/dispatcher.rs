mod error;
mod execution;
mod session;
mod state;

use std::path::Path;
use std::sync::Arc;

use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::{RwLock, watch};

use error::{binding_error, cancelled_result, command_meta, expired_result, session_error};
use state::{ActiveProject, DispatchState};

use crate::restart_handoff::RestartProject;

pub struct LocalProjectDispatcher {
    state: RwLock<DispatchState>,
}

#[derive(Debug, Error)]
pub enum ProjectBindingError {
    #[error(transparent)]
    File(#[from] crate::files::RemoteFileError),
    #[error("previous project session cleanup failed: {0}")]
    Cleanup(String),
}

impl LocalProjectDispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: RwLock::new(DispatchState::default()),
        }
    }

    pub async fn upsert(
        &self,
        thread_id: &str,
        root: impl AsRef<Path>,
        expires_at: DateTime<Utc>,
    ) -> Result<(), ProjectBindingError> {
        let project = Arc::new(ActiveProject::open(root.as_ref(), expires_at)?);
        let previous = self
            .state
            .write()
            .await
            .projects
            .insert(thread_id.to_owned(), project);
        if let Some(previous) = previous
            && let Some(session) = previous.session.lock().await.take()
        {
            session::close_session(&session)
                .await
                .map_err(ProjectBindingError::Cleanup)?;
        }
        Ok(())
    }

    pub async fn restart_projects(&self, now: DateTime<Utc>) -> Vec<RestartProject> {
        let state = self.state.read().await;
        let mut projects = state
            .projects
            .iter()
            .filter(|(_, project)| project.expires_at > now)
            .map(|(thread_id, project)| RestartProject {
                thread_id: thread_id.clone(),
                root: project.access.root().to_path_buf(),
                expires_at: project.expires_at,
            })
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
        projects
    }

    pub async fn restore_restart_projects(
        &self,
        projects: &[RestartProject],
    ) -> Result<(), ProjectBindingError> {
        for project in projects {
            self.upsert(&project.thread_id, &project.root, project.expires_at)
                .await?;
        }
        Ok(())
    }

    pub async fn execute(
        &self,
        command: GatewayCommand,
        connection_generation: Option<u64>,
    ) -> BridgeResult {
        let (keep_alive, cancelled) = watch::channel(false);
        let result = self
            .execute_cancellable(command, connection_generation, cancelled)
            .await;
        drop(keep_alive);
        result
    }

    pub async fn execute_cancellable(
        &self,
        command: GatewayCommand,
        connection_generation: Option<u64>,
        mut cancelled: watch::Receiver<bool>,
    ) -> BridgeResult {
        if *cancelled.borrow() {
            return cancelled_result(command.request_id());
        }
        if let GatewayCommand::DeviceSession {
            thread_id,
            working_directory,
            expires_at,
            request_id,
            ..
        } = &command
            && let Err(error) = self.upsert(thread_id, working_directory, *expires_at).await
        {
            return match error {
                ProjectBindingError::File(error) => error::file_error(request_id, &error),
                ProjectBindingError::Cleanup(message) => error::session_error(request_id, &message),
            };
        }
        let meta = command_meta(&command);
        let project = {
            self.state
                .read()
                .await
                .projects
                .get(meta.thread_id)
                .cloned()
        };
        let Some(project) = project else {
            return binding_error(meta.request_id, "binding_missing");
        };
        if meta.deadline_at <= Utc::now() {
            return expired_result(meta.request_id);
        }
        if project.expires_at <= Utc::now() {
            return binding_error(meta.request_id, "binding_expired");
        }
        if let Some((session_id, session_generation, computer_mode)) =
            state::session_activation(&command)
        {
            return self
                .activate(
                    &project,
                    meta.request_id,
                    session_id,
                    session_generation,
                    computer_mode,
                    connection_generation,
                )
                .await;
        }
        let session = match self
            .require_session(&project, meta.computer_session_id, connection_generation)
            .await
        {
            Ok(session) => session,
            Err(message) => return session_error(meta.request_id, &message),
        };
        if state::is_mutation(&command) {
            let mut mutations = tokio::select! {
                mutations = project.mutations.lock() => mutations,
                () = wait_for_cancellation(&mut cancelled) => {
                    return cancelled_result(meta.request_id);
                }
            };
            if meta.deadline_at <= Utc::now() {
                return expired_result(meta.request_id);
            }
            if let Some(result) = mutations.lookup(&command) {
                return result;
            }
            let result = execution::execute(&command, &project, &session, cancelled).await;
            mutations.remember(&command, &result);
            result
        } else {
            execution::execute(&command, &project, &session, cancelled).await
        }
    }
}

async fn wait_for_cancellation(cancelled: &mut watch::Receiver<bool>) {
    loop {
        if *cancelled.borrow() {
            return;
        }
        if cancelled.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl Default for LocalProjectDispatcher {
    fn default() -> Self {
        Self::new()
    }
}
