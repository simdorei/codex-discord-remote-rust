//! In-process admission fence used by the identity-bound restart drain protocol.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;
use tokio::sync::Notify;
use tokio::time::{Instant, timeout_at};

mod key;

pub use key::DrainFenceKey;

#[derive(Clone, Debug)]
pub struct AdmissionGate {
    inner: Arc<GateInner>,
}

#[derive(Debug)]
struct GateInner {
    state: Mutex<GateState>,
    changed: Notify,
}

#[derive(Debug, Default)]
struct GateState {
    active: usize,
    sealed: Option<DrainFenceKey>,
    controls_open: bool,
}

#[derive(Debug)]
pub struct AdmissionPermit {
    inner: Arc<GateInner>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DrainGateError {
    #[error("restart drain key is malformed")]
    InvalidKey,
    #[error("restart admission is sealed; retry after the runtime restarts")]
    Sealed,
    #[error("restart drain fence does not match the active runtime and nonce")]
    FenceMismatch,
    #[error("restart admission gate lock is poisoned")]
    LockPoisoned,
    #[error("restart admission drain timed out after {timeout_ms} ms")]
    Timeout { timeout_ms: u128 },
}

impl AdmissionGate {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(GateInner {
                state: Mutex::new(GateState::default()),
                changed: Notify::new(),
            }),
        }
    }

    pub fn try_enter(&self) -> Result<AdmissionPermit, DrainGateError> {
        self.try_enter_kind(false).map(|(permit, _)| permit)
    }

    pub fn try_enter_control(&self) -> Result<AdmissionPermit, DrainGateError> {
        self.try_enter_kind(true).map(|(permit, _)| permit)
    }

    pub(crate) fn try_enter_control_observed(
        &self,
    ) -> Result<(AdmissionPermit, bool), DrainGateError> {
        self.try_enter_kind(true)
    }

    fn try_enter_kind(&self, control: bool) -> Result<(AdmissionPermit, bool), DrainGateError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| DrainGateError::LockPoisoned)?;
        if state.sealed.is_some() && !(control && state.controls_open) {
            return Err(DrainGateError::Sealed);
        }
        state.active = state
            .active
            .checked_add(1)
            .ok_or(DrainGateError::LockPoisoned)?;
        let sealed = state.sealed.is_some();
        Ok((
            AdmissionPermit {
                inner: Arc::clone(&self.inner),
            },
            sealed,
        ))
    }

    pub fn seal(&self, key: &DrainFenceKey) -> Result<(), DrainGateError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| DrainGateError::LockPoisoned)?;
        match &state.sealed {
            None => {
                state.sealed = Some(key.clone());
                state.controls_open = true;
            }
            Some(active) if active == key => {}
            Some(_) => return Err(DrainGateError::FenceMismatch),
        }
        self.inner.changed.notify_waiters();
        Ok(())
    }

    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.inner
            .state
            .lock()
            .map_or(true, |state| state.sealed.is_some())
    }

    pub fn close_controls(&self, key: &DrainFenceKey) -> Result<(), DrainGateError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| DrainGateError::LockPoisoned)?;
        if state.sealed.as_ref() != Some(key) {
            return Err(DrainGateError::FenceMismatch);
        }
        state.controls_open = false;
        self.inner.changed.notify_waiters();
        Ok(())
    }

    pub fn open_controls(&self, key: &DrainFenceKey) -> Result<(), DrainGateError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| DrainGateError::LockPoisoned)?;
        if state.sealed.as_ref() != Some(key) {
            return Err(DrainGateError::FenceMismatch);
        }
        state.controls_open = true;
        self.inner.changed.notify_waiters();
        Ok(())
    }

    #[must_use]
    pub fn is_drained_for(&self, key: &DrainFenceKey) -> bool {
        self.inner
            .state
            .lock()
            .is_ok_and(|state| state.active == 0 && state.sealed.as_ref() == Some(key))
    }

    pub async fn wait_drained(
        &self,
        key: &DrainFenceKey,
        timeout: Duration,
    ) -> Result<(), DrainGateError> {
        let deadline = Instant::now() + timeout;
        loop {
            let changed = self.inner.changed.notified();
            {
                let state = self
                    .inner
                    .state
                    .lock()
                    .map_err(|_| DrainGateError::LockPoisoned)?;
                if state.sealed.as_ref() != Some(key) {
                    return Err(DrainGateError::FenceMismatch);
                }
                if state.active == 0 {
                    return Ok(());
                }
            }
            timeout_at(deadline, changed)
                .await
                .map_err(|_| DrainGateError::Timeout {
                    timeout_ms: timeout.as_millis(),
                })?;
        }
    }

    #[must_use]
    pub fn release(&self, key: &DrainFenceKey) -> bool {
        let Ok(mut state) = self.inner.state.lock() else {
            return false;
        };
        if state.active != 0 || state.sealed.as_ref() != Some(key) {
            return false;
        }
        state.sealed = None;
        state.controls_open = false;
        self.inner.changed.notify_waiters();
        true
    }
}

impl Default for AdmissionGate {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for AdmissionPermit {
    fn clone(&self) -> Self {
        if let Ok(mut state) = self.inner.state.lock() {
            state.active = state.active.saturating_add(1);
        }
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl PartialEq for AdmissionPermit {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for AdmissionPermit {}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.active = state.active.saturating_sub(1);
            if state.active == 0 {
                self.inner.changed.notify_waiters();
            }
        }
    }
}
