#[cfg(test)]
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static PROCESS_LAST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub(super) enum SequenceSource {
    Process,
    #[cfg(test)]
    Test(Arc<AtomicU64>),
}

impl SequenceSource {
    pub(super) fn next(&self) -> Option<u64> {
        let counter = match self {
            Self::Process => &PROCESS_LAST_SEQUENCE,
            #[cfg(test)]
            Self::Test(counter) => counter,
        };
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
                last.checked_add(1)
            })
            .ok()
            .map(|last| last + 1)
    }
}
