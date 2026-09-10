//! Runtime-owned side of the filesystem restart drain handshake.

use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio::time::{MissedTickBehavior, interval};
use uuid::Uuid;

use super::drain::{AdmissionGate, DrainFenceKey, DrainGateError};
use super::drain_marker::{
    self, DrainMarkerError, RuntimeMarker, publish_ack, publish_identity, read_prepare,
};
use super::live_drain::LiveDrainError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrainedTransition {
    Restart,
    Stop,
}

#[derive(Debug, Error)]
pub enum DrainProtocolError {
    #[error(transparent)]
    Marker(#[from] DrainMarkerError),
    #[error(transparent)]
    Gate(#[from] DrainGateError),
    #[error("orphaned restart drain state exists; watchdog must prove the former runtime exited")]
    OrphanedState,
    #[error("restart drain prepare targets a different runtime identity")]
    IdentityMismatch,
    #[error("restart drain prepare changed before acknowledgement")]
    PrepareChanged,
    #[error("restart drain controller received Ctrl+C while sealed: {0}")]
    CtrlC(std::io::Error),
    #[error(transparent)]
    LiveDrain(#[from] LiveDrainError),
    #[error("restart live drain timed out: {last_reason}")]
    LiveDrainTimeout { last_reason: String },
}

pub struct RuntimeDrainController {
    root: PathBuf,
    identity: RuntimeMarker,
    gate: AdmissionGate,
    untrusted_prepare_key: DrainFenceKey,
}

impl RuntimeDrainController {
    pub fn initialize(root: &Path) -> Result<Self, DrainProtocolError> {
        if drain_marker::prepare_path(root).exists() || drain_marker::ack_path(root).exists() {
            return Err(DrainProtocolError::OrphanedState);
        }
        let identity = RuntimeMarker {
            runtime_id: Uuid::new_v4().to_string(),
            process_id: std::process::id(),
        };
        let untrusted_prepare_key = DrainFenceKey::new(
            identity.runtime_id.clone(),
            format!("{}|0", identity.process_id),
            "untrusted-prepare",
        )?;
        publish_identity(root, &identity)?;
        Ok(Self {
            root: root.to_owned(),
            identity,
            gate: AdmissionGate::new(),
            untrusted_prepare_key,
        })
    }

    #[must_use]
    pub fn admission_gate(&self) -> AdmissionGate {
        self.gate.clone()
    }

    #[must_use]
    pub fn runtime_id(&self) -> &str {
        &self.identity.runtime_id
    }

    pub fn quarantine_untrusted_prepare(&self) -> Result<(), DrainProtocolError> {
        self.gate.seal(&self.untrusted_prepare_key)?;
        Ok(())
    }

    pub fn claim_prepare(&self) -> Result<Option<DrainFenceKey>, DrainProtocolError> {
        let Some(key) = read_prepare(&self.root)? else {
            return Ok(None);
        };
        let expected_pid = format!("{}|", self.identity.process_id);
        if key.runtime_id() != self.identity.runtime_id
            || !key.process_identity().starts_with(&expected_pid)
        {
            return Err(DrainProtocolError::IdentityMismatch);
        }
        self.gate.seal(&key)?;
        Ok(Some(key))
    }

    pub fn acknowledge(&self, key: &DrainFenceKey) -> Result<(), DrainProtocolError> {
        if !self.gate.is_drained_for(key) || read_prepare(&self.root)?.as_ref() != Some(key) {
            return Err(DrainProtocolError::PrepareChanged);
        }
        publish_ack(&self.root, key)?;
        if read_prepare(&self.root)?.as_ref() != Some(key) {
            drain_marker::remove_ack_if_owned(&self.root, key)?;
            return Err(DrainProtocolError::PrepareChanged);
        }
        Ok(())
    }

    pub async fn wait_for_transition(&self, key: &DrainFenceKey) -> DrainedTransition {
        self.wait_for_transition_inner(key, false).await
    }

    pub async fn complete_drained_handshake(&self, key: &DrainFenceKey) -> DrainedTransition {
        self.wait_for_transition_inner(key, true).await
    }

    async fn wait_for_transition_inner(
        &self,
        key: &DrainFenceKey,
        publish_acknowledgement: bool,
    ) -> DrainedTransition {
        let mut poll = interval(Duration::from_millis(100));
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let mut ctrl_c_enabled = true;
        let mut acknowledgement_complete = !publish_acknowledgement;
        let mut acknowledgement_failure = None;
        let mut restart_read_failure = None;
        loop {
            tokio::select! {
                _ = poll.tick() => {
                    if crate::operation_marker::stop_requested(&self.root) {
                        return DrainedTransition::Stop;
                    }
                    if !acknowledgement_complete {
                        match self.acknowledge(key) {
                            Ok(()) => {
                                acknowledgement_complete = true;
                                acknowledgement_failure = None;
                                eprintln!("rust_runtime_restart_drain_ack nonce={}", key.nonce());
                            }
                            Err(error) => {
                                report_changed_failure(
                                    &mut acknowledgement_failure,
                                    "restart_drain_acknowledgement_failed",
                                    &error,
                                );
                                continue;
                            }
                        }
                    }
                    match drain_marker::read_restart(&self.root) {
                        Ok(Some(candidate)) if &candidate == key => {
                            return DrainedTransition::Restart;
                        }
                        Ok(_) => restart_read_failure = None,
                        Err(error) => report_changed_failure(
                            &mut restart_read_failure,
                            "restart_drain_restart_marker_read_failed",
                            &error,
                        ),
                    }
                }
                signal = &mut ctrl_c, if ctrl_c_enabled => match signal {
                    Ok(()) => return DrainedTransition::Stop,
                    Err(error) => {
                        ctrl_c_enabled = false;
                        eprintln!("restart_drain_ctrl_c_listener_failed: {error}; runtime_remains_sealed=true");
                    }
                }
            }
        }
    }
}

fn report_changed_failure(
    previous: &mut Option<String>,
    event: &str,
    error: &impl std::fmt::Display,
) {
    let current = error.to_string();
    if previous.as_ref() != Some(&current) {
        eprintln!("{event}: {current}; runtime_remains_sealed=true");
        *previous = Some(current);
    }
}

impl Drop for RuntimeDrainController {
    fn drop(&mut self) {
        if let Err(error) = drain_marker::remove_identity_if_owned(&self.root, &self.identity) {
            eprintln!("restart_drain_identity_cleanup_failed: {error}");
        }
    }
}

#[cfg(test)]
#[path = "drain_controller_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "drain_controller_transition_tests.rs"]
mod transition_tests;
