use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::client::AdmissionPermit;
use crate::{AppServerClient, AppServerError, DeadGenerationWork};

#[path = "admission_replacement.rs"]
mod replacement;
#[path = "admission_restart.rs"]
mod restart;
#[path = "admission_terminal.rs"]
mod terminal;

#[derive(Clone)]
pub(super) struct ResidentState {
    pub(super) inner: Arc<Mutex<ResidentStateInner>>,
    restart_signal: watch::Sender<Option<u64>>,
}

pub(super) struct ResidentStateInner {
    pub(super) client: Option<AppServerClient>,
    replacement_cleanup: Option<ReplacementCleanup>,
    pub(super) generation: u64,
    quarantined: bool,
    pub(super) restart_pending: bool,
    pub(super) accepting: bool,
    pub(super) close_state: ResidentCloseState,
    pub(super) settled_dead_work: Option<DeadGenerationWork>,
    pub(super) cleanup_authorized_generation: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResidentCloseState {
    Open,
    Terminal,
}

pub(super) struct ResidentAdmission {
    pub(super) client: AppServerClient,
    pub(super) generation: u64,
    _permit: AdmissionPermit,
}

pub(super) struct WrittenRequestGuard<'a> {
    state: &'a ResidentState,
    generation: u64,
    write_started: bool,
    complete: bool,
}

pub(super) struct ResidentStateSnapshot {
    pub(super) client: Option<AppServerClient>,
    pub(super) generation: u64,
    pub(super) quarantined: bool,
    pub(super) restart_pending: bool,
    pub(super) accepting: bool,
}

#[derive(Clone)]
pub(super) struct ReplacementCleanup {
    pub(super) client: AppServerClient,
    pub(super) generation: u64,
}

pub(super) enum RestartCandidate {
    NotPending,
    Busy,
    Sealed(AppServerClient),
}

impl ResidentState {
    pub(super) fn new(client: AppServerClient) -> Self {
        let (restart_signal, _) = watch::channel(None);
        Self {
            inner: Arc::new(Mutex::new(ResidentStateInner {
                client: Some(client),
                replacement_cleanup: None,
                generation: 1,
                quarantined: false,
                restart_pending: false,
                accepting: true,
                close_state: ResidentCloseState::Open,
                settled_dead_work: None,
                cleanup_authorized_generation: None,
            })),
            restart_signal,
        }
    }

    pub(super) fn admit_request(
        &self,
        expected_generation: Option<u64>,
    ) -> Result<ResidentAdmission, AppServerError> {
        self.admit(expected_generation, true)
    }

    pub(super) fn admit_response(
        &self,
        expected_generation: u64,
    ) -> Result<ResidentAdmission, AppServerError> {
        self.admit(Some(expected_generation), false)
    }

    fn admit(
        &self,
        expected_generation: Option<u64>,
        reject_quarantined: bool,
    ) -> Result<ResidentAdmission, AppServerError> {
        let state = self.inner.lock().expect("resident state lock");
        if let Some(expected) = expected_generation
            && expected != state.generation
        {
            return Err(AppServerError::GenerationMismatch {
                expected,
                actual: state.generation,
            });
        }
        if !state.accepting {
            return Err(AppServerError::Closed);
        }
        if reject_quarantined && state.quarantined {
            return Err(AppServerError::GenerationQuarantined {
                generation: state.generation,
            });
        }
        let client = state.client.as_ref().ok_or(AppServerError::Closed)?;
        let permit = client.admit_operation()?;
        Ok(ResidentAdmission {
            client: client.clone(),
            generation: state.generation,
            _permit: permit,
        })
    }

    pub(super) fn track_written_request(&self, generation: u64) -> WrittenRequestGuard<'_> {
        WrittenRequestGuard {
            state: self,
            generation,
            write_started: false,
            complete: false,
        }
    }

    pub(super) fn current_client(&self) -> Result<AppServerClient, AppServerError> {
        let state = self.inner.lock().expect("resident state lock");
        if !state.accepting {
            return Err(AppServerError::Closed);
        }
        state.client.clone().ok_or(AppServerError::Closed)
    }

    pub(super) fn snapshot(&self) -> ResidentStateSnapshot {
        let state = self.inner.lock().expect("resident state lock");
        ResidentStateSnapshot {
            client: state.client.clone(),
            generation: state.generation,
            quarantined: state.quarantined,
            restart_pending: state.restart_pending,
            accepting: state.accepting,
        }
    }

    pub(super) fn generation(&self) -> u64 {
        self.inner.lock().expect("resident state lock").generation
    }
}

impl WrittenRequestGuard<'_> {
    pub(super) fn confirm_write_started(&mut self) {
        self.write_started = true;
    }

    fn mark_ambiguous(&self) {
        if self.write_started {
            self.state.mark_cancelled(self.generation);
        }
    }

    pub(super) fn finish<T>(&mut self, result: &Result<T, AppServerError>) {
        if matches!(
            result,
            Err(AppServerError::Io(_)
                | AppServerError::Closed
                | AppServerError::TransportClosed { .. }
                | AppServerError::ResponseChannelClosed { .. })
        ) {
            self.mark_ambiguous();
        }
        self.complete = true;
    }
}

impl Drop for WrittenRequestGuard<'_> {
    fn drop(&mut self) {
        if self.write_started && !self.complete {
            self.state.mark_cancelled(self.generation);
        }
    }
}
