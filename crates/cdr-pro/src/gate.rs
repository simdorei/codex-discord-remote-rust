use std::collections::HashMap;
use std::path::Path;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateMode {
    Open,
    Hold,
    Discard,
}

#[derive(Debug, Default)]
struct GateState {
    modes: HashMap<String, GateMode>,
    discard_sizes: HashMap<String, u64>,
}

#[derive(Debug, Default)]
pub struct SessionMirrorGate {
    state: Mutex<GateState>,
    opened: Condvar,
}

impl SessionMirrorGate {
    pub fn hold(&self, thread_id: Option<&str>) {
        let Some(key) = target_key(thread_id) else {
            return;
        };
        let mut state = self.lock();
        state.modes.insert(key.clone(), GateMode::Hold);
        state.discard_sizes.remove(&key);
    }

    pub fn approve(&self, thread_id: Option<&str>) {
        self.clear(thread_id);
    }

    pub fn reject(&self, thread_id: Option<&str>) {
        let Some(key) = target_key(thread_id) else {
            return;
        };
        let mut state = self.lock();
        state.modes.insert(key.clone(), GateMode::Discard);
        state.discard_sizes.remove(&key);
    }

    #[must_use]
    pub fn mode(&self, thread_id: Option<&str>) -> GateMode {
        let Some(key) = target_key(thread_id) else {
            return GateMode::Open;
        };
        self.lock()
            .modes
            .get(&key)
            .copied()
            .unwrap_or(GateMode::Open)
    }

    #[must_use]
    pub fn wait_until_open(&self, thread_id: Option<&str>, timeout: Duration) -> bool {
        let Some(key) = target_key(thread_id) else {
            return true;
        };
        let started = Instant::now();
        let mut state = self.lock();
        while state.modes.contains_key(&key) {
            let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                return false;
            };
            let waited = self.opened.wait_timeout(state, remaining);
            let (next, result) = waited.unwrap_or_else(PoisonError::into_inner);
            state = next;
            if result.timed_out() && state.modes.contains_key(&key) {
                return false;
            }
        }
        true
    }

    #[must_use]
    pub fn discard_size_is_stable(&self, thread_id: Option<&str>, size: u64) -> bool {
        let Some(key) = target_key(thread_id) else {
            return false;
        };
        let mut state = self.lock();
        if state.modes.get(&key) != Some(&GateMode::Discard) {
            return false;
        }
        let previous = state.discard_sizes.insert(key, size);
        previous == Some(size)
    }

    pub fn finish_discard(&self, thread_id: Option<&str>) {
        self.clear(thread_id);
    }

    fn clear(&self, thread_id: Option<&str>) {
        let Some(key) = target_key(thread_id) else {
            return;
        };
        let mut state = self.lock();
        state.modes.remove(&key);
        state.discard_sizes.remove(&key);
        drop(state);
        self.opened.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn target_key(thread_id: Option<&str>) -> Option<String> {
    thread_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn gate_rollout_output<F>(
    gate: &SessionMirrorGate,
    thread_id: &str,
    rollout_path: &Path,
    update_cursor: F,
) -> std::result::Result<bool, String>
where
    F: FnOnce(&str, &Path, u64) -> std::result::Result<(), String>,
{
    match gate.mode(Some(thread_id)) {
        GateMode::Open => return Ok(false),
        GateMode::Hold => return Ok(true),
        GateMode::Discard => {}
    }
    let Ok(metadata) = rollout_path.metadata() else {
        return Ok(true);
    };
    let size = metadata.len();
    if !gate.discard_size_is_stable(Some(thread_id), size) {
        return Ok(true);
    }
    update_cursor(thread_id, rollout_path, size)?;
    if !matches!(rollout_path.metadata(), Ok(metadata) if metadata.len() == size) {
        return Ok(true);
    }
    gate.finish_discard(Some(thread_id));
    Ok(true)
}
