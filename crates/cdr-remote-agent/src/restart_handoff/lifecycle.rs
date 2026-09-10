use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{
    HandoffProtector, RESUME_ENV_NAME, RestartHandoffError, SystemProtector,
    claim_restart_handoff_at, write_restart_handoff_at,
};
use crate::config::RemoteMcpConfig;
use crate::dispatcher::{LocalProjectDispatcher, ProjectBindingError};

#[derive(Clone)]
pub struct RestartHandoffRuntime {
    path: PathBuf,
    protector: Arc<dyn HandoffProtector>,
    resume_requested: bool,
}

#[derive(Debug, Error)]
pub enum RestartHandoffLifecycleError {
    #[error(transparent)]
    Handoff(#[from] RestartHandoffError),
    #[error(transparent)]
    Binding(#[from] ProjectBindingError),
}

impl RestartHandoffRuntime {
    #[must_use]
    pub fn new(
        path: impl Into<PathBuf>,
        protector: Arc<dyn HandoffProtector>,
        resume_requested: bool,
    ) -> Self {
        Self {
            path: path.into(),
            protector,
            resume_requested,
        }
    }

    pub fn system(repo_root: &Path) -> Result<Self, RestartHandoffError> {
        Ok(Self::new(
            restart_handoff_path(repo_root)?,
            Arc::new(SystemProtector),
            resume_requested(),
        ))
    }

    pub async fn prepare(
        &self,
        dispatcher: &LocalProjectDispatcher,
        config: &RemoteMcpConfig,
        now: DateTime<Utc>,
    ) -> Result<bool, RestartHandoffLifecycleError> {
        let projects = dispatcher.restart_projects(now).await;
        write_restart_handoff_at(&projects, config, &self.path, self.protector.as_ref(), now)
            .map_err(Into::into)
    }

    pub async fn restore(
        &self,
        dispatcher: &LocalProjectDispatcher,
        config: &RemoteMcpConfig,
        now: DateTime<Utc>,
    ) -> Result<usize, RestartHandoffLifecycleError> {
        let projects = claim_restart_handoff_at(
            config,
            &self.path,
            self.protector.as_ref(),
            now,
            self.resume_requested,
        )?;
        dispatcher.restore_restart_projects(&projects).await?;
        Ok(projects.len())
    }
}

fn restart_handoff_path(repo_root: &Path) -> Result<PathBuf, RestartHandoffError> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|path| path.join(".local/state"))
        })
        .ok_or(RestartHandoffError::LocalStateUnavailable)?;
    let canonical = repo_root.canonicalize()?;
    let normalized = canonical.to_string_lossy().to_lowercase();
    let digest = format!("{:x}", Sha256::digest(normalized.as_bytes()));
    Ok(base
        .join("simdorei/codex-discord-remote")
        .join(&digest[..16])
        .join("remote-mcp-restart.json"))
}

fn resume_requested() -> bool {
    std::env::var(RESUME_ENV_NAME).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}
