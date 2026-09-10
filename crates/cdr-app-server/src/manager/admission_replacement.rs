use std::sync::Arc;

use super::{ReplacementCleanup, ResidentCloseState, ResidentState};
use crate::{AppServerClient, AppServerError};

impl ResidentState {
    pub(in crate::manager) fn replacement_generation(&self) -> u64 {
        self.inner
            .lock()
            .expect("resident state lock")
            .generation
            .checked_add(1)
            .expect("resident generation overflow")
    }

    pub(in crate::manager) fn record_replacement(
        &self,
        client: &AppServerClient,
        generation: u64,
    ) -> Result<(), AppServerError> {
        let mut state = self.inner.lock().expect("resident state lock");
        if state.close_state != ResidentCloseState::Open {
            return Err(AppServerError::Closed);
        }
        let expected = state
            .generation
            .checked_add(1)
            .expect("resident generation overflow");
        if generation != expected {
            return Err(replacement_error(format!(
                "candidate generation {generation} does not follow {expected}"
            )));
        }
        if state.replacement_cleanup.is_some() {
            return Err(replacement_error(
                "another replacement still requires cleanup".to_owned(),
            ));
        }
        state.replacement_cleanup = Some(ReplacementCleanup {
            client: client.clone(),
            generation,
        });
        Ok(())
    }

    pub(in crate::manager) fn replacement_cleanup(&self) -> Option<ReplacementCleanup> {
        self.inner
            .lock()
            .expect("resident state lock")
            .replacement_cleanup
            .clone()
    }

    #[cfg(test)]
    pub(in crate::manager) fn has_replacement_cleanup(&self) -> bool {
        self.inner
            .lock()
            .expect("resident state lock")
            .replacement_cleanup
            .is_some()
    }

    pub(in crate::manager) fn finish_replacement_cleanup(
        &self,
        expected: &ReplacementCleanup,
    ) -> Result<(), AppServerError> {
        let mut state = self.inner.lock().expect("resident state lock");
        let Some(actual) = state.replacement_cleanup.as_ref() else {
            return Err(replacement_error(
                "replacement cleanup debt is missing".to_owned(),
            ));
        };
        if !same_replacement(actual, expected) {
            return Err(replacement_error(
                "replacement cleanup debt changed identity".to_owned(),
            ));
        }
        state.replacement_cleanup = None;
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::manager) fn install_replacement(
        &self,
        expected: &AppServerClient,
        generation: u64,
    ) -> Result<(), AppServerError> {
        self.install_replacement_with(expected, generation, || Ok(()))
    }

    pub(in crate::manager) fn install_replacement_with<F>(
        &self,
        expected: &AppServerClient,
        generation: u64,
        install: F,
    ) -> Result<(), AppServerError>
    where
        F: FnOnce() -> Result<(), AppServerError>,
    {
        let mut state = self.inner.lock().expect("resident state lock");
        if state.close_state != ResidentCloseState::Open {
            return Err(AppServerError::Closed);
        }
        let Some(candidate) = state.replacement_cleanup.as_ref() else {
            return Err(replacement_error(
                "replacement candidate is missing".to_owned(),
            ));
        };
        let next = state
            .generation
            .checked_add(1)
            .expect("resident generation overflow");
        if candidate.generation != generation
            || generation != next
            || !Arc::ptr_eq(&candidate.client.inner, &expected.inner)
        {
            return Err(replacement_error(
                "replacement candidate identity or generation changed".to_owned(),
            ));
        }
        expected.inner.lifecycle.with_open(|| {
            install()?;
            let candidate = state
                .replacement_cleanup
                .take()
                .expect("validated replacement candidate");
            state.client = Some(candidate.client);
            state.generation = generation;
            state.cleanup_authorized_generation = None;
            state.quarantined = false;
            self.set_restart_pending_locked(&mut state, false);
            state.accepting = true;
            Ok(())
        })?
    }
}

fn same_replacement(left: &ReplacementCleanup, right: &ReplacementCleanup) -> bool {
    left.generation == right.generation && Arc::ptr_eq(&left.client.inner, &right.client.inner)
}

fn replacement_error(message: String) -> AppServerError {
    AppServerError::ReplacementState { message }
}
