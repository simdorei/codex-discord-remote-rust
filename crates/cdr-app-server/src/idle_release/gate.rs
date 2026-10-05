use super::{ExclusivePermit, IdleReleaseJournal, IdleReleaseToken, held};
use crate::AppServerError;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(test)]
mod observation_tests;

#[derive(Default)]
struct State {
    mutations: HashMap<Option<String>, usize>,
    exclusive: HashSet<String>,
    journal: Option<Arc<dyn IdleReleaseJournal>>,
}

#[derive(Default)]
pub(crate) struct TargetGate {
    state: Mutex<State>,
    observation_gap: AtomicU64,
}

pub(crate) enum MutationAdmission {
    Ordinary(MutationPermit),
    Resubscribe(ExclusivePermit, IdleReleaseToken),
}

pub(crate) struct MutationPermit {
    gate: Arc<TargetGate>,
    target: Option<String>,
}

impl TargetGate {
    pub(crate) fn install(
        &self,
        journal: Arc<dyn IdleReleaseJournal>,
    ) -> Result<(), AppServerError> {
        let mut state = self.state.lock().expect("target gate");
        if state.journal.is_some() || !state.mutations.is_empty() || !state.exclusive.is_empty() {
            return Err(held("idle journal must be installed once before intake"));
        }
        state.journal = Some(journal);
        Ok(())
    }

    pub(crate) fn journal(&self) -> Option<Arc<dyn IdleReleaseJournal>> {
        self.state.lock().expect("target gate").journal.clone()
    }

    pub(crate) fn admit(
        self: &Arc<Self>,
        owner: &str,
        generation: u64,
        target: Option<String>,
    ) -> Result<MutationAdmission, AppServerError> {
        let mut state = self.state.lock().expect("target gate");
        if target
            .as_ref()
            .map_or(!state.exclusive.is_empty(), |t| state.exclusive.contains(t))
        {
            return Err(held(
                "target subscription maintenance is in flight; request not sent",
            ));
        }
        if let (Some(thread), Some(journal)) = (target.as_ref(), state.journal.clone())
            && let Some(token) = journal.before_mutation(owner, generation, thread)?
        {
            // AwaitUnload cannot coexist with an admitted mutation: its release
            // reservation was exclusive until ACK persistence. Fail closed if violated.
            if state.mutations.contains_key(&None) || state.mutations.contains_key(&target) {
                return Err(held(
                    "resubscription admission conflicted; durable hold retained",
                ));
            }
            state.exclusive.insert(thread.clone());
            return Ok(MutationAdmission::Resubscribe(
                ExclusivePermit {
                    gate: Arc::clone(self),
                    thread: thread.clone(),
                    journal,
                },
                token,
            ));
        }
        *state.mutations.entry(target.clone()).or_default() += 1;
        Ok(MutationAdmission::Ordinary(MutationPermit {
            gate: Arc::clone(self),
            target,
        }))
    }

    pub(crate) fn reserve(
        self: &Arc<Self>,
        token: &IdleReleaseToken,
    ) -> Result<ExclusivePermit, AppServerError> {
        let mut state = self.state.lock().expect("target gate");
        if state.mutations.contains_key(&None)
            || state.mutations.contains_key(&Some(token.thread_id.clone()))
            || state.exclusive.contains(&token.thread_id)
        {
            return Err(held("idle release deferred: admitted target mutation"));
        }
        let journal = state
            .journal
            .clone()
            .ok_or_else(|| held("idle journal is not installed"))?;
        journal.verify(token, token.state == "Candidate")?;
        state.exclusive.insert(token.thread_id.clone());
        Ok(ExclusivePermit {
            gate: Arc::clone(self),
            thread: token.thread_id.clone(),
            journal,
        })
    }

    pub(crate) fn release_exclusive(&self, thread: &str) {
        self.state
            .lock()
            .expect("target gate")
            .exclusive
            .remove(thread);
    }

    pub(crate) fn mark_gap(&self) {
        let _ = self
            .observation_gap
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(value.saturating_add(1 + (value & 1)))
            });
    }
    // An unattributed loss has no source interval that a later prefix can prove.
    // Latch before its DB write; even an in-flight/failed write must keep idle held.
    pub(crate) fn hold_unattributed_gap(&self) {
        self.observation_gap.store(u64::MAX, Ordering::Release);
    }

    pub(crate) fn observations_verified(&self) -> bool {
        self.gap_epoch() & 1 == 0
    }
    pub(crate) fn gap_epoch(&self) -> u64 {
        self.observation_gap.load(Ordering::Acquire)
    }
    pub(crate) fn clear_gap(&self, expected: u64) -> bool {
        if expected == u64::MAX {
            return false;
        }
        if expected & 1 == 0 {
            return self.gap_epoch() == expected;
        }
        self.observation_gap
            .compare_exchange(expected, expected + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

impl MutationPermit {
    pub(crate) fn preflight(&self) -> Result<(), AppServerError> {
        let state = self.gate.state.lock().expect("target gate");
        if let (Some(journal), Some(target)) = (&state.journal, &self.target) {
            journal.check_mutation(target)?;
        }
        Ok(())
    }
}

impl Drop for MutationPermit {
    fn drop(&mut self) {
        let mut state = self.gate.state.lock().expect("target gate");
        let count = state
            .mutations
            .get_mut(&self.target)
            .expect("counted mutation");
        *count -= 1;
        if *count == 0 {
            state.mutations.remove(&self.target);
        }
    }
}
