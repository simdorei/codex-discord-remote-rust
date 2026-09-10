use super::ResidentAppServer;
use super::admission::{ResidentCloseState, ResidentState};
use crate::{AppServerError, DeadGenerationSettleResult, DeadGenerationWork};

impl ResidentAppServer {
    pub(super) async fn fence_dead_generation_before_restart(
        &self,
    ) -> Result<bool, AppServerError> {
        let Some(fence) = self.dead_generation_fence.as_ref() else {
            return Ok(true);
        };
        let generation = self.state.generation();
        let snapshot = self.state.snapshot();
        if !snapshot.restart_pending {
            return Ok(true);
        }
        let client = snapshot.client.ok_or(AppServerError::Closed)?;
        // Continue a previously sealed, quiescent client's intentional cleanup;
        // this is not a claim that an arbitrary missing child handle means dead.
        if self.state.restart_cleanup_authorized(generation) {
            return Ok(true);
        }
        let exited = {
            let mut child = client.inner.child.lock().await;
            match child.as_mut() {
                Some(child) => child.try_wait()?.is_some(),
                None => return Ok(false),
            }
        };
        let closure_published = client.lifecycle_snapshot().closed_reason.is_some();
        if !exited {
            // A live timeout quarantine can drain normally. Transport
            // cancellation/EOF alone cannot authorize settling live work.
            return Ok(!closure_published);
        }
        // OS exit can precede the reader's final messages and closure signal.
        // Wait for that publication so empty/Starting-only work is fenced too.
        if !closure_published || !client.inner.lifecycle.seal_if_quiescent(|| true) {
            return Ok(false);
        }
        // The sealed lifecycle now has zero admitted operations. An already
        // written response can no longer resolve/remove an occurrence while
        // the exact snapshot is being persisted and compared below.
        let Some(work) = self.state.dead_generation_work_for_fence(generation)? else {
            return Ok(true);
        };
        fence.persist(&work)?;
        match self.state.settle_dead_generation(generation, &work)? {
            DeadGenerationSettleResult::Settled | DeadGenerationSettleResult::AlreadySettled => {
                Ok(true)
            }
            DeadGenerationSettleResult::NotEligible
            | DeadGenerationSettleResult::SnapshotChanged => {
                Err(AppServerError::DeadGenerationFence {
                    message: "dead-generation snapshot changed after durable capture".into(),
                })
            }
        }
    }

    pub async fn dead_generation_work(
        &self,
        expected_generation: u64,
    ) -> Result<Option<DeadGenerationWork>, AppServerError> {
        let _guard = self.restart_lock.lock().await;
        self.state.dead_generation_work(expected_generation)
    }

    pub async fn settle_dead_generation(
        &self,
        expected_generation: u64,
        expected: &DeadGenerationWork,
    ) -> Result<DeadGenerationSettleResult, AppServerError> {
        let _guard = self.restart_lock.lock().await;
        self.state
            .settle_dead_generation(expected_generation, expected)
    }
}

impl ResidentState {
    fn dead_generation_work_for_fence(
        &self,
        generation: u64,
    ) -> Result<Option<DeadGenerationWork>, AppServerError> {
        let mut state = self.inner.lock().expect("resident state lock");
        validate_generation(&state, generation)?;
        if state.close_state != ResidentCloseState::Open || !state.restart_pending {
            return Ok(None);
        }
        if let Some(settled) = &state.settled_dead_work
            && settled.generation == generation
        {
            return Ok(Some(settled.clone()));
        }
        let client = state.client.clone().ok_or(AppServerError::Closed)?;
        let work = client
            .inner
            .state
            .lock()
            .expect("runtime state lock")
            .dead_generation_work(generation);
        if work.is_some() {
            state.accepting = false;
        }
        Ok(work)
    }

    fn dead_generation_work(
        &self,
        expected_generation: u64,
    ) -> Result<Option<DeadGenerationWork>, AppServerError> {
        let state = self.inner.lock().expect("resident state lock");
        validate_generation(&state, expected_generation)?;
        if !eligible(&state) {
            return Ok(None);
        }
        let client = state.client.as_ref().ok_or(AppServerError::Closed)?;
        let work = client
            .inner
            .state
            .lock()
            .expect("runtime state lock")
            .dead_generation_work(expected_generation)
            .filter(|work| !work.is_empty());
        Ok(work)
    }

    fn settle_dead_generation(
        &self,
        expected_generation: u64,
        expected: &DeadGenerationWork,
    ) -> Result<DeadGenerationSettleResult, AppServerError> {
        let mut state = self.inner.lock().expect("resident state lock");
        if expected.generation != expected_generation {
            return Ok(DeadGenerationSettleResult::SnapshotChanged);
        }
        if state.settled_dead_work.as_ref() == Some(expected) {
            return Ok(DeadGenerationSettleResult::AlreadySettled);
        }
        validate_generation(&state, expected_generation)?;
        if !eligible(&state) {
            return Ok(DeadGenerationSettleResult::NotEligible);
        }
        let client = state.client.clone().ok_or(AppServerError::Closed)?;
        let mut runtime = client.inner.state.lock().expect("runtime state lock");
        let Some(actual) = runtime.dead_generation_work(expected_generation) else {
            return Ok(DeadGenerationSettleResult::NotEligible);
        };
        if actual != *expected {
            return Ok(DeadGenerationSettleResult::SnapshotChanged);
        }
        runtime.settle_dead_generation_after_exact_match();
        state.settled_dead_work = Some(expected.clone());
        Ok(DeadGenerationSettleResult::Settled)
    }
}

fn validate_generation(
    state: &super::admission::ResidentStateInner,
    expected: u64,
) -> Result<(), AppServerError> {
    if state.generation == expected {
        Ok(())
    } else {
        Err(AppServerError::GenerationMismatch {
            expected,
            actual: state.generation,
        })
    }
}

fn eligible(state: &super::admission::ResidentStateInner) -> bool {
    state.close_state == ResidentCloseState::Open
        && state.restart_pending
        && !state.accepting
        && state
            .client
            .as_ref()
            .is_some_and(|client| client.lifecycle_snapshot().closed_reason.is_some())
}
