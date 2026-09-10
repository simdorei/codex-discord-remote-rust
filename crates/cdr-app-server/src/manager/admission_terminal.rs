use std::sync::Arc;

use super::{ReplacementCleanup, ResidentCloseState, ResidentState};
use crate::{AppServerClient, AppServerError};

pub(in crate::manager) struct ResidentClosePlan {
    pub(in crate::manager) current: Option<AppServerClient>,
    pub(in crate::manager) replacement: Option<ReplacementCleanup>,
}

impl ResidentState {
    pub(in crate::manager) fn prepare_close(&self) -> ResidentClosePlan {
        let mut state = self.inner.lock().expect("resident state lock");
        state.accepting = false;
        state.close_state = ResidentCloseState::Terminal;
        self.set_restart_pending_locked(&mut state, false);
        let current = state.client.clone();
        let replacement = state.replacement_cleanup.clone();
        if let Some(client) = current.as_ref() {
            client.seal_admissions();
        }
        if let Some(cleanup) = replacement.as_ref() {
            cleanup.client.seal_admissions();
        }
        ResidentClosePlan {
            current,
            replacement,
        }
    }

    pub(in crate::manager) fn finish_current_close(
        &self,
        expected: &AppServerClient,
    ) -> Result<(), AppServerError> {
        let mut state = self.inner.lock().expect("resident state lock");
        if state.close_state != ResidentCloseState::Terminal {
            return Err(AppServerError::ReplacementState {
                message: "resident close is no longer terminal".to_owned(),
            });
        }
        let Some(current) = state.client.as_ref() else {
            return Ok(());
        };
        if !Arc::ptr_eq(&current.inner, &expected.inner) {
            return Err(AppServerError::ReplacementState {
                message: "resident close target changed identity".to_owned(),
            });
        }
        state.client = None;
        Ok(())
    }
}
