use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use twilight_model::id::{Id, marker::InteractionMarker};

const DEFAULT_CAPACITY: usize = 4_096;

/// Bounded process-local memory of Discord interactions claimed for
/// acknowledgement or already acknowledged.
///
/// Clone this cache into every dispatcher instance that belongs to the same
/// gateway loop. It prevents a Discord redelivery from enqueueing the same
/// work twice. A pending guard may be released after acknowledgement failure,
/// but the durable custody journal still suppresses later execution.
/// Pending claims are never evicted; a cache containing only pending claims
/// rejects additional claims until one is committed or released.
#[derive(Clone, Debug)]
pub struct InteractionClaimCache {
    inner: Arc<Mutex<ClaimState>>,
}

#[derive(Debug)]
struct ClaimState {
    capacity: usize,
    next_generation: u128,
    claims: HashMap<u64, ClaimEntry>,
    committed_order: VecDeque<InteractionClaimToken>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClaimEntry {
    generation: u128,
    state: ClaimStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaimStatus {
    Pending,
    Committed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InteractionClaimToken {
    interaction_id: u64,
    generation: u128,
}

#[derive(Debug)]
pub(super) struct InteractionClaim {
    cache: InteractionClaimCache,
    token: Option<InteractionClaimToken>,
}

#[derive(Debug)]
pub(super) enum InteractionClaimAttempt {
    Claimed(InteractionClaim),
    DuplicatePending,
    DuplicateCommitted,
    Saturated,
}

impl Default for InteractionClaimCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl InteractionClaimCache {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClaimState {
                capacity: capacity.max(1),
                next_generation: 1,
                claims: HashMap::new(),
                committed_order: VecDeque::new(),
            })),
        }
    }

    pub(super) fn try_claim(
        &self,
        interaction_id: Id<InteractionMarker>,
    ) -> InteractionClaimAttempt {
        let mut state = self.state();
        let interaction_id = interaction_id.get();
        if let Some(existing) = state.claims.get(&interaction_id) {
            return match existing.state {
                ClaimStatus::Pending => InteractionClaimAttempt::DuplicatePending,
                ClaimStatus::Committed => InteractionClaimAttempt::DuplicateCommitted,
            };
        }
        while state.claims.len() >= state.capacity {
            if !state.evict_oldest_committed() {
                return InteractionClaimAttempt::Saturated;
            }
        }
        let token = InteractionClaimToken {
            interaction_id,
            generation: state.next_generation,
        };
        state.next_generation += 1;
        state.claims.insert(
            interaction_id,
            ClaimEntry {
                generation: token.generation,
                state: ClaimStatus::Pending,
            },
        );
        InteractionClaimAttempt::Claimed(InteractionClaim {
            cache: self.clone(),
            token: Some(token),
        })
    }

    fn commit(&self, token: InteractionClaimToken) -> bool {
        let mut state = self.state();
        let Some(entry) = state.claims.get_mut(&token.interaction_id) else {
            return false;
        };
        if entry.generation != token.generation || entry.state != ClaimStatus::Pending {
            return false;
        }
        entry.state = ClaimStatus::Committed;
        state.committed_order.push_back(token);
        true
    }

    fn release(&self, token: InteractionClaimToken) -> bool {
        let mut state = self.state();
        let should_release = state
            .claims
            .get(&token.interaction_id)
            .is_some_and(|entry| {
                entry.generation == token.generation && entry.state == ClaimStatus::Pending
            });
        if should_release {
            state.claims.remove(&token.interaction_id);
        }
        should_release
    }

    fn state(&self) -> MutexGuard<'_, ClaimState> {
        self.inner
            .lock()
            .expect("interaction claim cache mutex poisoned")
    }
}

impl InteractionClaim {
    pub(super) fn commit(mut self) -> bool {
        let Some(token) = self.token else {
            return false;
        };
        if !self.cache.commit(token) {
            return false;
        }
        self.token = None;
        true
    }
}

impl Drop for InteractionClaim {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            self.cache.release(token);
        }
    }
}

impl ClaimState {
    fn evict_oldest_committed(&mut self) -> bool {
        while let Some(token) = self.committed_order.pop_front() {
            let matches = self.claims.get(&token.interaction_id).is_some_and(|entry| {
                entry.generation == token.generation && entry.state == ClaimStatus::Committed
            });
            if matches {
                self.claims.remove(&token.interaction_id);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_claims_are_not_evicted_and_stale_release_cannot_erase_a_new_claim() {
        let cache = InteractionClaimCache::new(2);
        let InteractionClaimAttempt::Claimed(first) = cache.try_claim(Id::new(1)) else {
            panic!("first interaction should be claimed");
        };
        let InteractionClaimAttempt::Claimed(second) = cache.try_claim(Id::new(2)) else {
            panic!("second interaction should be claimed");
        };
        assert!(matches!(
            cache.try_claim(Id::new(3)),
            InteractionClaimAttempt::Saturated
        ));
        assert!(second.commit());
        let InteractionClaimAttempt::Claimed(third) = cache.try_claim(Id::new(3)) else {
            panic!("a committed claim should be evictable");
        };
        assert_eq!(cache.state().claims.len(), 2);
        assert!(!cache.state().claims.contains_key(&2));
        assert!(matches!(
            cache.try_claim(Id::new(1)),
            InteractionClaimAttempt::DuplicatePending
        ));
        let stale_token = first.token.expect("claimed guard should hold its token");
        assert!(cache.release(stale_token));
        let InteractionClaimAttempt::Claimed(replacement) = cache.try_claim(Id::new(1)) else {
            panic!("released interaction should be claimable again");
        };
        drop(first);
        assert!(matches!(
            cache.try_claim(Id::new(1)),
            InteractionClaimAttempt::DuplicatePending
        ));
        drop(third);
        let InteractionClaimAttempt::Claimed(reclaimed_eviction) = cache.try_claim(Id::new(2))
        else {
            panic!("the evicted committed interaction should be claimable again");
        };
        assert!(replacement.commit());
        assert!(reclaimed_eviction.commit());
        assert!(matches!(
            cache.try_claim(Id::new(1)),
            InteractionClaimAttempt::DuplicateCommitted
        ));
    }

    #[test]
    fn poisoned_claim_state_is_not_silently_recovered() {
        let cache = InteractionClaimCache::new(1);
        let poisoner = cache.clone();
        assert!(
            std::panic::catch_unwind(move || {
                let _guard = poisoner.inner.lock().unwrap();
                panic!("poison claim state");
            })
            .is_err()
        );
        assert!(std::panic::catch_unwind(|| cache.try_claim(Id::new(1))).is_err());
    }
}
