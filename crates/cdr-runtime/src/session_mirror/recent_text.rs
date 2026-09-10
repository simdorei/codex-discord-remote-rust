use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

type TextDigest = [u8; 32];
type SeenText = HashMap<String, HashMap<TextDigest, Instant>>;

pub(crate) struct RecentTextCache {
    ttl: Duration,
    seen: Mutex<SeenText>,
}

impl RecentTextCache {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            seen: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn is_recent(&self, thread_id: &str, text: &str) -> bool {
        self.is_recent_at(thread_id, text, Instant::now())
    }

    pub(crate) fn remember(&self, thread_id: &str, text: &str) {
        self.remember_at(thread_id, text, Instant::now());
    }

    pub(crate) fn prune_expired(&self) {
        prune_seen(&mut self.lock_seen(), Instant::now(), self.ttl);
    }

    fn is_recent_at(&self, thread_id: &str, text: &str, now: Instant) -> bool {
        let digest = normalized_text_digest(text);
        let mut seen = self.lock_seen();
        prune_seen(&mut seen, now, self.ttl);
        seen.get(thread_id)
            .is_some_and(|thread| thread.contains_key(&digest))
    }

    fn remember_at(&self, thread_id: &str, text: &str, now: Instant) {
        let mut seen = self.lock_seen();
        prune_seen(&mut seen, now, self.ttl);
        let thread_seen = seen.entry(thread_id.to_owned()).or_default();
        thread_seen.insert(normalized_text_digest(text), now);
    }

    fn lock_seen(&self) -> MutexGuard<'_, SeenText> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn prune_seen(seen: &mut SeenText, now: Instant, ttl: Duration) {
    seen.retain(|_, thread| {
        thread.retain(|_, seen_at| elapsed(now, *seen_at) <= ttl);
        !thread.is_empty()
    });
}

pub(crate) fn normalized_text_digest(text: &str) -> TextDigest {
    let mut digest = Sha256::new();
    digest.update(text.trim().as_bytes());
    digest.update([0]);
    digest.finalize().into()
}

fn elapsed(now: Instant, seen_at: Instant) -> Duration {
    now.checked_duration_since(seen_at).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_trims_text_isolates_threads_and_expires_after_ttl() {
        let ttl = Duration::from_secs(10);
        let cache = RecentTextCache::new(ttl);
        let first = Instant::now();
        cache.remember_at("one", "  update  ", first);

        assert!(cache.is_recent_at("one", "update", first + ttl));
        assert!(!cache.is_recent_at("two", "update", first + ttl));
        assert!(!cache.is_recent_at("one", "update", first + ttl + Duration::from_nanos(1)));
    }

    #[test]
    fn remembering_another_thread_removes_expired_historical_threads() {
        let cache = RecentTextCache::new(Duration::from_secs(1));
        let first = Instant::now();
        for index in 0..100 {
            cache.remember_at(&format!("old-{index}"), "finished", first);
        }
        cache.remember_at("fresh", "new", first + Duration::from_secs(2));
        let seen = cache.lock_seen();
        assert_eq!(seen.len(), 1);
        assert!(seen.contains_key("fresh"));
    }

    #[test]
    fn checking_an_unknown_thread_also_releases_expired_text_and_outer_map_entry() {
        let cache = RecentTextCache::new(Duration::from_secs(1));
        let first = Instant::now();
        cache.remember_at("old", "finished", first);
        assert!(!cache.is_recent_at("unknown", "different", first + Duration::from_secs(2)));
        assert!(cache.lock_seen().is_empty());
    }
}
