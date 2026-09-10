//! Fail-closed restart readiness using Rust-owned durable state and read-only app-server calls.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cdr_app_server::requests::read_thread_with_timeout;
use cdr_app_server::{AppServerClient, AppServerConfig, AppServerError};
use cdr_store::restart_readiness::snapshot;
use thiserror::Error;
use tokio::time::{Instant, sleep, timeout};

use crate::runtime_paths::RuntimePaths;

pub mod drain;
pub mod drain_controller;
pub(crate) mod drain_marker;
pub(crate) mod live_drain;
pub mod maintenance;
mod thread_state;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RestartReadinessState {
    Ready,
    Blocked { reason: String },
}

#[derive(Clone, Copy, Debug)]
pub struct RestartReadinessOptions {
    pub quiet: Duration,
    pub wait_timeout: Duration,
    pub poll_interval: Duration,
    pub request_timeout: Duration,
    pub startup_timeout: Duration,
    pub close_timeout: Duration,
}

#[derive(Debug, Error)]
pub enum RestartReadinessError {
    #[error(transparent)]
    Store(#[from] cdr_store::StoreError),
    #[error(transparent)]
    AppServer(#[from] AppServerError),
    #[error("restart readiness app-server startup timed out after {timeout_ms} ms")]
    StartupTimeout { timeout_ms: u128 },
    #[error("restart readiness app-server close timed out after {timeout_ms} ms")]
    CloseTimeout { timeout_ms: u128 },
    #[error("restart readiness system clock is before the Unix epoch: {0}")]
    SystemClock(#[from] std::time::SystemTimeError),
    #[error("invalid app-server thread state for {thread_id}: {reason}")]
    InvalidThreadState { thread_id: String, reason: String },
    #[error("restart readiness timed out: {last_reason}")]
    DeadlineExceeded { last_reason: String },
    #[error("configured CODEX_HOME is not valid Unicode: {0}")]
    InvalidCodexHome(String),
}

pub async fn wait_for_restart(
    paths: &RuntimePaths,
    options: RestartReadinessOptions,
) -> Result<(), RestartReadinessError> {
    let codex_home = paths.codex_home.to_str().ok_or_else(|| {
        RestartReadinessError::InvalidCodexHome(paths.codex_home.display().to_string())
    })?;
    let config = AppServerConfig::new(&paths.codex_exe)
        .with_environment([("CODEX_HOME".to_owned(), codex_home.to_owned())]);
    wait_for_restart_with_config(&paths.mirror_db, config, options).await
}

pub async fn wait_for_restart_with_config(
    mirror_db: &Path,
    config: AppServerConfig,
    options: RestartReadinessOptions,
) -> Result<(), RestartReadinessError> {
    let client = timeout(options.startup_timeout, AppServerClient::start(config))
        .await
        .map_err(|_| RestartReadinessError::StartupTimeout {
            timeout_ms: options.startup_timeout.as_millis(),
        })??;
    let outcome = wait_with_client(mirror_db, &client, options).await;
    let close = timeout(options.close_timeout, client.close()).await;
    match close {
        Err(_) => Err(RestartReadinessError::CloseTimeout {
            timeout_ms: options.close_timeout.as_millis(),
        }),
        Ok(Err(error)) => Err(error.into()),
        Ok(Ok(())) => outcome,
    }
}

pub async fn check_restart_readiness(
    mirror_db: &Path,
    client: &AppServerClient,
    quiet: Duration,
    request_timeout: Duration,
) -> Result<RestartReadinessState, RestartReadinessError> {
    check_readiness_inner(mirror_db, client, quiet, request_timeout, None).await
}

async fn check_readiness_inner(
    mirror_db: &Path,
    client: &AppServerClient,
    quiet: Duration,
    request_timeout: Duration,
    absent: Option<&maintenance::AbsentTarget>,
) -> Result<RestartReadinessState, RestartReadinessError> {
    let proof = absent.map(|ticket| ticket.prove(mirror_db)).transpose()?;
    let before = snapshot(mirror_db)?;
    if let Some(reason) = before.blockers.first() {
        return Ok(RestartReadinessState::Blocked {
            reason: reason.clone(),
        });
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    for thread_id in &before.target_thread_ids {
        if absent.is_some_and(|ticket| ticket.thread_id == *thread_id) {
            continue;
        }
        let request = read_thread_with_timeout(thread_id, false, request_timeout);
        let result = client
            .request(request.method, request.params, request.timeout)
            .await?;
        let state = thread_state::classify(&result, thread_id, quiet, now)?;
        if let RestartReadinessState::Blocked { .. } = state {
            return Ok(state);
        }
    }
    let after = snapshot(mirror_db)?;
    if proof != absent.map(|ticket| ticket.prove(mirror_db)).transpose()? {
        return Ok(RestartReadinessState::Blocked {
            reason: "approved absence changed during maintenance inspection".into(),
        });
    }
    if before != after {
        return Ok(RestartReadinessState::Blocked {
            reason: "Rust durable restart state changed during app-server inspection".into(),
        });
    }
    Ok(RestartReadinessState::Ready)
}

async fn wait_with_client(
    mirror_db: &Path,
    client: &AppServerClient,
    options: RestartReadinessOptions,
) -> Result<(), RestartReadinessError> {
    let started = Instant::now();
    loop {
        let state =
            check_restart_readiness(mirror_db, client, options.quiet, options.request_timeout)
                .await?;
        let RestartReadinessState::Blocked { reason } = state else {
            return Ok(());
        };
        let elapsed = started.elapsed();
        if elapsed >= options.wait_timeout {
            return Err(RestartReadinessError::DeadlineExceeded {
                last_reason: reason,
            });
        }
        let remaining = options.wait_timeout.saturating_sub(elapsed);
        sleep(options.poll_interval.min(remaining)).await;
    }
}
