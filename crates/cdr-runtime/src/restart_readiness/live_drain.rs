//! In-process quiescence check performed before the live app-server is closed.

use std::collections::BTreeMap;
use std::path::Path;

use cdr_app_server::{AppServerError, ResidentAppServer, ResidentLifecycleSnapshot};
use cdr_store::restart_readiness::{RestartReadinessSnapshot, snapshot};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LiveDrainState {
    Ready,
    Blocked { reason: String },
}

#[derive(Debug, Error)]
pub enum LiveDrainError {
    #[error(transparent)]
    Store(#[from] cdr_store::StoreError),
    #[error(transparent)]
    AppServer(#[from] AppServerError),
}

pub(crate) trait LiveDrainServer {
    async fn lifecycle(&self) -> ResidentLifecycleSnapshot;
    async fn active_turn(&self, thread_id: &str) -> Result<Option<String>, AppServerError>;
    async fn has_unsettled_requests(&self) -> Result<bool, AppServerError>;
}

impl LiveDrainServer for ResidentAppServer {
    async fn lifecycle(&self) -> ResidentLifecycleSnapshot {
        self.lifecycle_snapshot().await
    }

    async fn active_turn(&self, thread_id: &str) -> Result<Option<String>, AppServerError> {
        self.active_turn_id(thread_id).await
    }

    async fn has_unsettled_requests(&self) -> Result<bool, AppServerError> {
        self.has_unsettled_server_requests().await
    }
}

pub(crate) async fn check_runtime_quiescence(
    mirror_db: &Path,
    server: &impl LiveDrainServer,
) -> Result<LiveDrainState, LiveDrainError> {
    let durable_before = snapshot(mirror_db)?;
    if let Some(reason) = durable_before.blockers.first() {
        return Ok(LiveDrainState::Blocked {
            reason: reason.clone(),
        });
    }
    let live_before = observe(server, &durable_before).await?;
    if let Some(reason) = live_before.blocker() {
        return Ok(LiveDrainState::Blocked { reason });
    }
    let durable_after = snapshot(mirror_db)?;
    if durable_before != durable_after {
        return Ok(LiveDrainState::Blocked {
            reason: "Rust durable restart state changed during live drain inspection".into(),
        });
    }
    let live_after = observe(server, &durable_after).await?;
    if live_before != live_after {
        return Ok(LiveDrainState::Blocked {
            reason: "resident app-server state changed during live drain inspection".into(),
        });
    }
    Ok(LiveDrainState::Ready)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LiveObservation {
    lifecycle: ResidentLifecycleSnapshot,
    unsettled: bool,
    active: BTreeMap<String, String>,
}

impl LiveObservation {
    fn blocker(&self) -> Option<String> {
        if !self.lifecycle.healthy
            || self.lifecycle.quarantined
            || self.lifecycle.restart_pending
            || self.lifecycle.process_id.is_none()
        {
            return Some(format!(
                "resident app-server is not stably healthy: {:?}",
                self.lifecycle
            ));
        }
        if self.unsettled {
            return Some("resident app-server has an unsettled approval or input request".into());
        }
        self.active.iter().next().map(|(thread, turn)| {
            format!("resident app-server thread {thread} still has active turn {turn}")
        })
    }
}

async fn observe(
    server: &impl LiveDrainServer,
    durable: &RestartReadinessSnapshot,
) -> Result<LiveObservation, AppServerError> {
    let lifecycle = server.lifecycle().await;
    let unsettled = server.has_unsettled_requests().await?;
    let mut active = BTreeMap::new();
    for thread in &durable.target_thread_ids {
        if let Some(turn) = server.active_turn(thread).await? {
            active.insert(thread.clone(), turn);
        }
    }
    Ok(LiveObservation {
        lifecycle,
        unsettled,
        active,
    })
}

#[cfg(test)]
#[path = "live_drain_tests.rs"]
mod tests;
