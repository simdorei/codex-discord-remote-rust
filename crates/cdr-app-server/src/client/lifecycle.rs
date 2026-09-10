use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

use crate::AppServerError;

#[derive(Debug, Default)]
struct LifecycleState {
    close_intent: Option<String>,
    in_flight: usize,
    sealed: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ClientLifecycle {
    closed_reason: watch::Sender<Option<String>>,
    state: Arc<Mutex<LifecycleState>>,
}

#[derive(Debug)]
#[must_use = "dropping the permit ends the admitted operation"]
pub(crate) struct AdmissionPermit {
    state: Arc<Mutex<LifecycleState>>,
}

impl Default for ClientLifecycle {
    fn default() -> Self {
        let (closed_reason, _) = watch::channel(None);
        Self {
            closed_reason,
            state: Arc::new(Mutex::new(LifecycleState::default())),
        }
    }
}

impl ClientLifecycle {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn admit(&self) -> Result<AdmissionPermit, AppServerError> {
        let mut state = self.state.lock().expect("client lifecycle lock");
        if state.sealed {
            return Err(AppServerError::Closed);
        }
        state.in_flight = state
            .in_flight
            .checked_add(1)
            .expect("client admission count overflow");
        Ok(AdmissionPermit {
            state: Arc::clone(&self.state),
        })
    }

    pub(crate) fn seal(&self) {
        self.state.lock().expect("client lifecycle lock").sealed = true;
    }

    pub(crate) fn seal_for_close(&self, reason: &str) {
        let mut state = self.state.lock().expect("client lifecycle lock");
        state.sealed = true;
        if state.close_intent.is_none() {
            state.close_intent = Some(reason.to_owned());
        }
    }

    pub(crate) fn seal_and_resolve_close_reason(&self, observed_reason: &str) -> String {
        let mut state = self.state.lock().expect("client lifecycle lock");
        state.sealed = true;
        state
            .close_intent
            .clone()
            .unwrap_or_else(|| observed_reason.to_owned())
    }

    pub(crate) fn publish_closed(&self, reason: String) {
        self.closed_reason.send_if_modified(move |current| {
            if current.is_some() {
                return false;
            }
            *current = Some(reason);
            true
        });
    }

    pub(crate) async fn wait_closed(&self) -> String {
        let mut closed_reason = self.closed_reason.subscribe();
        loop {
            if let Some(reason) = closed_reason.borrow_and_update().clone() {
                return reason;
            }
            closed_reason
                .changed()
                .await
                .expect("client lifecycle retains close signal sender");
        }
    }

    pub(crate) fn seal_if_quiescent<F>(&self, check: F) -> bool
    where
        F: FnOnce() -> bool,
    {
        let mut state = self.state.lock().expect("client lifecycle lock");
        if state.in_flight != 0 || !check() {
            return false;
        }
        state.sealed = true;
        true
    }

    pub(crate) fn with_open<F, R>(&self, action: F) -> Result<R, AppServerError>
    where
        F: FnOnce() -> R,
    {
        let state = self.state.lock().expect("client lifecycle lock");
        if state.sealed {
            return Err(AppServerError::Closed);
        }
        Ok(action())
    }

    #[cfg(test)]
    fn is_sealed(&self) -> bool {
        self.state.lock().expect("client lifecycle lock").sealed
    }

    #[cfg(test)]
    fn in_flight(&self) -> usize {
        self.state.lock().expect("client lifecycle lock").in_flight
    }
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.in_flight = state
            .in_flight
            .checked_sub(1)
            .expect("client admission count underflow");
    }
}

#[cfg(test)]
#[path = "lifecycle_close_tests.rs"]
mod close_signal_tests;

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::TryLockError;

    use super::ClientLifecycle;
    use crate::AppServerError;

    #[test]
    fn admitted_operations_block_seal_until_every_permit_drops() {
        let lifecycle = ClientLifecycle::new();
        let first = lifecycle.admit().expect("admit first operation");
        let second = lifecycle.admit().expect("admit second operation");

        let mut checked = false;
        assert!(!lifecycle.seal_if_quiescent(|| {
            checked = true;
            true
        }));
        assert!(!checked);
        assert!(!lifecycle.is_sealed());
        assert_eq!(lifecycle.in_flight(), 2);

        drop(first);
        assert_eq!(lifecycle.in_flight(), 1);
        assert!(!lifecycle.seal_if_quiescent(|| true));

        drop(second);
        assert_eq!(lifecycle.in_flight(), 0);
        assert!(lifecycle.seal_if_quiescent(|| true));
        assert!(lifecycle.is_sealed());
    }

    #[test]
    fn false_tentative_check_leaves_gate_observably_open() {
        let lifecycle = ClientLifecycle::new();
        let state = lifecycle.state.clone();

        assert!(!lifecycle.seal_if_quiescent(|| {
            assert!(matches!(state.try_lock(), Err(TryLockError::WouldBlock)));
            false
        }));
        assert!(!lifecycle.is_sealed());
        assert_eq!(
            lifecycle.with_open(|| "still open").expect("open gate"),
            "still open"
        );
        assert!(lifecycle.admit().is_ok());
    }

    #[test]
    fn seal_rejects_admission_and_open_only_work() {
        let lifecycle = ClientLifecycle::new();
        lifecycle.seal();

        let mut checked = false;
        assert!(!lifecycle.seal_if_quiescent(|| {
            checked = true;
            false
        }));
        assert!(checked);

        assert!(matches!(lifecycle.admit(), Err(AppServerError::Closed)));
        let mut ran = false;
        assert!(matches!(
            lifecycle.with_open(|| {
                ran = true;
            }),
            Err(AppServerError::Closed)
        ));
        assert!(!ran);
    }

    #[test]
    fn with_open_executes_while_holding_the_gate() {
        let lifecycle = ClientLifecycle::new();
        let state = lifecycle.state.clone();

        lifecycle
            .with_open(|| {
                assert!(matches!(state.try_lock(), Err(TryLockError::WouldBlock)));
            })
            .expect("open gate");
    }

    #[test]
    fn permit_drop_decrements_after_an_open_action_panics() {
        let lifecycle = ClientLifecycle::new();
        let permit = lifecycle.admit().expect("admit operation");

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _ = lifecycle.with_open(|| panic!("intentional gate panic"));
        }));
        assert!(panic.is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| drop(permit))).is_ok());

        let poisoned = lifecycle.state.lock().expect_err("poisoned lifecycle lock");
        assert_eq!(poisoned.into_inner().in_flight, 0);
    }
}
