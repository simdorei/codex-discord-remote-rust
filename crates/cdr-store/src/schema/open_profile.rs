//! Opt-in debug-build measurements for temporary test DB opens, not runtime state.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::Location;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Counters {
    calls: u64,
    misses: u64,
    total: Duration,
    connection: Duration,
    catalog: Duration,
    cache: Duration,
    initialization: Duration,
    maximum: Duration,
}

struct Summary {
    rows: BTreeMap<(&'static str, u32), Counters>,
    omitted: u64,
}

thread_local! {
    static SUMMARY: RefCell<Summary> = const {
        RefCell::new(Summary { rows: BTreeMap::new(), omitted: 0 })
    };
}

impl Drop for Summary {
    fn drop(&mut self) {
        for ((file, line), row) in &self.rows {
            eprintln!(
                "store_open_profile test={:?} caller={file}:{line} calls={} misses={} total_us={} connection_us={} catalog_us={} cache_us={} initialization_us={} maximum_us={}",
                std::thread::current().name(),
                row.calls,
                row.misses,
                row.total.as_micros(),
                row.connection.as_micros(),
                row.catalog.as_micros(),
                row.cache.as_micros(),
                row.initialization.as_micros(),
                row.maximum.as_micros(),
            );
        }
        if self.omitted != 0 {
            eprintln!("store_open_profile omitted_call_sites={}", self.omitted);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Phase {
    Connection,
    Catalog,
    Cache,
}

struct Measurement {
    location: &'static Location<'static>,
    start: Instant,
    last: Instant,
    connection: Duration,
    catalog: Duration,
    cache: Duration,
    miss: bool,
}

pub(super) struct Span(Option<Measurement>);

impl Span {
    pub(super) fn new(location: &'static Location<'static>) -> Self {
        let enabled = std::env::var("CDR_TEST_STORE_OPEN_PROFILE").is_ok_and(|filter| {
            filter.is_empty()
                || std::thread::current()
                    .name()
                    .is_some_and(|name| name.contains(&filter))
        });
        Self(enabled.then(|| {
            let now = Instant::now();
            Measurement {
                location,
                start: now,
                last: now,
                connection: Duration::ZERO,
                catalog: Duration::ZERO,
                cache: Duration::ZERO,
                miss: false,
            }
        }))
    }

    pub(super) fn mark(&mut self, phase: Phase) {
        if let Some(measurement) = &mut self.0 {
            let now = Instant::now();
            let duration = now.duration_since(measurement.last);
            match phase {
                Phase::Connection => measurement.connection = duration,
                Phase::Catalog => measurement.catalog = duration,
                Phase::Cache => measurement.cache = duration,
            }
            measurement.last = now;
        }
    }

    pub(super) fn miss(&mut self) {
        if let Some(measurement) = &mut self.0 {
            measurement.miss = true;
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(measurement) = self.0.take() else {
            return;
        };
        let elapsed = measurement.start.elapsed();
        let remaining = measurement.last.elapsed();
        let _ = SUMMARY.try_with(|summary| {
            let Ok(mut summary) = summary.try_borrow_mut() else {
                return;
            };
            let key = (measurement.location.file(), measurement.location.line());
            if summary.rows.len() >= 128 && !summary.rows.contains_key(&key) {
                summary.omitted = summary.omitted.saturating_add(1);
                return;
            }
            let row = summary.rows.entry(key).or_default();
            row.calls = row.calls.saturating_add(1);
            row.misses = row.misses.saturating_add(u64::from(measurement.miss));
            row.total = row.total.saturating_add(elapsed);
            row.connection = row.connection.saturating_add(measurement.connection);
            row.catalog = row.catalog.saturating_add(measurement.catalog);
            row.cache = row.cache.saturating_add(measurement.cache);
            if measurement.miss {
                row.initialization = row.initialization.saturating_add(remaining);
            }
            row.maximum = row.maximum.max(elapsed);
        });
    }
}
