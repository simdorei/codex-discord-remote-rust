use std::sync::Arc;

use super::admission::{ResidentCloseState, ResidentState};
use super::events::ResidentForwarders;
use super::events::activation::await_activation;
use crate::AppServerClient;

impl ResidentState {
    pub(super) fn mark_current_closed(&self, expected: &AppServerClient, generation: u64) {
        let mut state = self.inner.lock().expect("resident state lock");
        if state.close_state != ResidentCloseState::Open || state.generation != generation {
            return;
        }
        let Some(current) = state.client.as_ref() else {
            return;
        };
        if !Arc::ptr_eq(&current.inner, &expected.inner) {
            return;
        }
        state.accepting = false;
        self.set_restart_pending_locked(&mut state, true);
    }
}

impl ResidentForwarders {
    pub(super) fn with_death_monitor(
        mut self,
        client: &AppServerClient,
        state: ResidentState,
        generation: u64,
    ) -> Self {
        let client = client.clone();
        let mut generation_rx = self.generation_rx.clone();
        let activation_rx = self.activation.subscribe();
        self.handles.push(tokio::spawn(async move {
            if !await_activation(activation_rx).await {
                return;
            }
            loop {
                if *generation_rx.borrow() != generation {
                    return;
                }
                tokio::select! {
                    _ = client.wait_closed() => {
                        state.mark_current_closed(&client, generation);
                        return;
                    }
                    changed = generation_rx.changed() => {
                        if changed.is_err() || *generation_rx.borrow() != generation {
                            return;
                        }
                    }
                }
            }
        }));
        self
    }
}
