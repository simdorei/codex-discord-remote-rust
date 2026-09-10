use tokio::sync::watch;

use super::{ResidentCloseState, ResidentState, ResidentStateInner, RestartCandidate};
use crate::AppServerError;

impl ResidentState {
    pub(in crate::manager) fn mark_timeout(&self, generation: u64) {
        let mut state = self.inner.lock().expect("resident state lock");
        if state.generation == generation && state.accepting && state.client.is_some() {
            state.quarantined = true;
            self.set_restart_pending_locked(&mut state, true);
        }
    }

    pub(in crate::manager) fn mark_cancelled(&self, generation: u64) {
        let mut state = self.inner.lock().expect("resident state lock");
        if state.generation == generation && state.accepting && state.client.is_some() {
            state.quarantined = true;
            self.set_restart_pending_locked(&mut state, true);
        }
    }

    pub(in crate::manager) fn request_restart(&self) {
        let mut state = self.inner.lock().expect("resident state lock");
        if state.close_state == ResidentCloseState::Open {
            self.set_restart_pending_locked(&mut state, true);
        }
    }

    pub(in crate::manager) fn restart_pending_for(&self, expected_generation: u64) -> bool {
        let state = self.inner.lock().expect("resident state lock");
        state.close_state == ResidentCloseState::Open
            && state.generation == expected_generation
            && state.restart_pending
    }

    pub(in crate::manager) fn restart_candidate(
        &self,
        expected_generation: Option<u64>,
    ) -> Result<RestartCandidate, AppServerError> {
        let mut state = self.inner.lock().expect("resident state lock");
        if state.close_state == ResidentCloseState::Terminal
            || expected_generation.is_some_and(|expected| expected != state.generation)
            || !state.restart_pending
        {
            return Ok(RestartCandidate::NotPending);
        }
        let client = state.client.clone().ok_or(AppServerError::Closed)?;
        if !client.seal_if_quiescent() {
            return Ok(RestartCandidate::Busy);
        }
        state.accepting = false;
        // The exact current client is now sealed with no admitted operations
        // or unresolved work. Keep this cleanup authorization across cancelled
        // or failed replacement startup, including after its child was reaped.
        state.cleanup_authorized_generation = Some(state.generation);
        Ok(RestartCandidate::Sealed(client))
    }

    pub(in crate::manager) fn restart_cleanup_authorized(&self, generation: u64) -> bool {
        let state = self.inner.lock().expect("resident state lock");
        state.close_state == ResidentCloseState::Open
            && state.generation == generation
            && !state.accepting
            && state.cleanup_authorized_generation == Some(generation)
    }

    pub(in crate::manager) fn subscribe_restart_pending(&self) -> watch::Receiver<Option<u64>> {
        self.restart_signal.subscribe()
    }

    pub(in crate::manager) fn set_restart_pending_locked(
        &self,
        state: &mut ResidentStateInner,
        pending: bool,
    ) {
        state.restart_pending = pending;
        let requested = pending.then_some(state.generation);
        self.restart_signal.send_replace(requested);
    }
}
