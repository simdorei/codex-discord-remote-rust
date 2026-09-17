use super::{MirrorSyncError, MirrorSynchronizer};
use cdr_codex_state::{CodexThreadStore, ThreadInfo};
use cdr_store::mapping::mirror_targets;
use std::collections::{BTreeMap, BTreeSet};

impl MirrorSynchronizer {
    pub(super) fn scope(&self, limit: Option<i64>) -> Result<Vec<ThreadInfo>, MirrorSyncError> {
        let store = CodexThreadStore::open(&self.state_db)?;
        let all = store
            .load_recent_threads(0)?
            .into_iter()
            .map(|thread| (thread.id.clone(), thread))
            .collect::<BTreeMap<_, _>>();
        let mut candidates = match limit {
            Some(limit) => {
                store.load_recent_threads(u32::try_from(limit.clamp(1, 100)).expect("bounded"))?
            }
            None => store.load_mirror_root_threads(0)?,
        };
        // Keep active bot-owned forks that are already mapped, even though their source is app-server.
        if limit.is_none() {
            for mapping in mirror_targets(&self.mirror_db, i64::MAX)? {
                if let Some(thread) = all.get(&mapping.codex_thread_id) {
                    candidates.push(thread.clone());
                }
            }
        }
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        for candidate in candidates {
            let id = candidate.id.clone();
            let thread = all.get(&id).ok_or_else(|| {
                MirrorSyncError::Invalid(format!("mapped fork {id} is unavailable"))
            })?;
            if seen.insert(id) {
                result.push(thread.clone());
            }
        }
        result.sort_by_key(|thread| thread.updated_at);
        Ok(result)
    }
}
