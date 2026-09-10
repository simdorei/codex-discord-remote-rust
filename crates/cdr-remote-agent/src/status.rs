use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Clone, Debug, Default)]
pub struct RemoteAgentStatus {
    inner: Arc<StatusInner>,
}

#[derive(Debug, Default)]
struct StatusInner {
    connected: AtomicBool,
    generation: AtomicU64,
}

impl RemoteAgentStatus {
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }

    pub(crate) fn set_connected(&self, generation: u64) {
        self.inner.generation.store(generation, Ordering::Release);
        self.inner.connected.store(true, Ordering::Release);
    }

    pub(crate) fn set_disconnected(&self) {
        self.inner.connected.store(false, Ordering::Release);
    }
}
