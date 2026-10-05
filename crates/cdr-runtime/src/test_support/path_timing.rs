//! Opt-in timing for local test fixtures; no payload or credentials are logged.
use std::time::Instant;

pub(crate) struct PhaseTimer {
    label: &'static str,
    start: Option<Instant>,
    last: Option<Instant>,
}

impl PhaseTimer {
    pub(crate) fn new(label: &'static str) -> Self {
        let start = std::env::var_os("CDR_TEST_PATH_TIMING").map(|_| Instant::now());
        Self {
            label,
            start,
            last: start,
        }
    }

    pub(crate) fn mark(&mut self, phase: &str) {
        if let (Some(start), Some(last)) = (self.start, self.last) {
            let now = Instant::now();
            eprintln!(
                "path_timing test={:?} label={} phase={phase} delta_us={} total_us={}",
                std::thread::current().name(),
                self.label,
                now.duration_since(last).as_micros(),
                now.duration_since(start).as_micros(),
            );
            self.last = Some(now);
        }
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        self.mark("finished_or_dropped");
    }
}
