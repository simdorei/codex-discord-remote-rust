use std::collections::BTreeMap;
use std::sync::Mutex;

/// First-turn knowledge is local to a live app-server generation, never durable
/// permission to replay a request. Consume before writing turn/start so ambiguous
/// outcomes must use normal history reconciliation, not an invented empty history.
pub(super) struct FreshThreads(Mutex<BTreeMap<String, u64>>);

impl FreshThreads {
    pub const fn new() -> Self {
        Self(Mutex::new(BTreeMap::new()))
    }

    pub fn remember(&self, thread: &str, generation: u64) {
        let mut entries = self.0.lock().expect("fresh thread registry poisoned");
        entries.retain(|_, stored| *stored == generation);
        entries.insert(thread.to_owned(), generation);
    }

    pub fn contains(&self, thread: &str, generation: u64) -> bool {
        self.0
            .lock()
            .expect("fresh thread registry poisoned")
            .get(thread)
            == Some(&generation)
    }

    pub fn consume(&self, thread: &str) {
        self.0
            .lock()
            .expect("fresh thread registry poisoned")
            .remove(thread);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_turn_knowledge_does_not_survive_a_generation_change_or_start_attempt() {
        let fresh = FreshThreads::new();
        fresh.remember("new", 7);
        assert!(fresh.contains("new", 7));
        assert!(!fresh.contains("new", 8));
        assert!(!fresh.contains("other", 7));
        fresh.consume("new");
        assert!(!fresh.contains("new", 7));
    }
}
