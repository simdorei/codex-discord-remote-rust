use std::collections::{HashSet, VecDeque};

pub const MAX_COMMITTED_REQUESTS: usize = 512;

#[derive(Debug, Default)]
pub struct SentRequestCache {
    generation: Option<u64>,
    keys: HashSet<String>,
    order: VecDeque<String>,
}

impl SentRequestCache {
    pub fn contains(&mut self, generation: u64, key: &str) -> bool {
        self.select_generation(generation);
        self.keys.contains(key)
    }

    pub fn remember(&mut self, generation: u64, key: String) {
        self.select_generation(generation);
        if self.keys.insert(key.clone()) {
            self.order.push_back(key);
        }
        while self.order.len() > MAX_COMMITTED_REQUESTS {
            if let Some(oldest) = self.order.pop_front() {
                self.keys.remove(&oldest);
            }
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    fn select_generation(&mut self, generation: u64) {
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.keys.clear();
            self.order.clear();
        }
    }
}
