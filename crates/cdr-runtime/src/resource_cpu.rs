//! CPU sampling sequence, separated from the unrelated host probes and clock.
use cdr_windows_native::{NativeError, resources::CpuTimes};
use std::time::Duration;

pub(super) struct Sample {
    pub percent: Result<f64, NativeError>,
    pub elapsed: Option<Duration>,
}

impl Sample {
    fn failure(error: NativeError) -> Self {
        Self {
            percent: Err(error),
            elapsed: None,
        }
    }
}

fn invalid_clock() -> NativeError {
    NativeError::InvalidInput("CPU sampling clock moved backwards".into())
}

pub(super) fn sample<T>(
    mut counters: impl FnMut() -> Result<CpuTimes, NativeError>,
    collect_other: impl FnOnce() -> T,
    clock: impl Fn() -> Duration,
    mut sleep: impl FnMut(Duration),
) -> (Sample, T) {
    let before = counters();
    // The first acquisition may itself block/deschedule for longer than the
    // minimum. Its setup latency must not consume the interval between samples.
    let start = clock();
    let other = collect_other();
    let before = match before {
        Ok(before) => before,
        Err(error) => return (Sample::failure(error), other),
    };
    let Some(waited) = clock().checked_sub(start) else {
        return (Sample::failure(invalid_clock()), other);
    };
    if let Some(remaining) = Duration::from_millis(200).checked_sub(waited) {
        sleep(remaining);
    }
    let after = counters();
    let end = clock();
    let measured = after.and_then(|after| {
        let elapsed = end.checked_sub(start).ok_or_else(invalid_clock)?;
        Ok((after.busy_percent_since(before)?, elapsed))
    });
    let sample = match measured {
        Ok((percent, elapsed)) => Sample {
            percent: Ok(percent),
            elapsed: Some(elapsed),
        },
        Err(error) => Sample::failure(error),
    };
    (sample, other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn successful_sequence(initial_delay: u64) {
        let now = Cell::new(Duration::ZERO);
        let calls = Cell::new(0);
        let waits = RefCell::new(Vec::new());
        let (sample, other) = sample(
            || {
                let index = calls.get();
                calls.set(index + 1);
                now.set(
                    now.get() + Duration::from_millis(if index == 0 { initial_delay } else { 10 }),
                );
                Ok(CpuTimes {
                    idle: if index == 0 { 0 } else { 30 },
                    kernel: if index == 0 { 0 } else { 80 },
                    user: if index == 0 { 0 } else { 20 },
                })
            },
            || {
                now.set(now.get() + Duration::from_millis(30));
                "other probes preserved"
            },
            || now.get(),
            |duration| {
                waits.borrow_mut().push(duration);
                now.set(now.get() + duration);
            },
        );
        assert_eq!(*waits.borrow(), [Duration::from_millis(170)]);
        assert_eq!(sample.elapsed, Some(Duration::from_millis(210)));
        assert!((sample.percent.unwrap() - 70.0).abs() < f64::EPSILON);
        assert_eq!(calls.get(), 2);
        assert_eq!(other, "other probes preserved");
    }

    #[test]
    fn first_counter_delay_does_not_consume_or_inflate_the_sample_interval() {
        successful_sequence(350);
    }

    #[test]
    fn normal_sample_preserves_the_interval_and_counter_arithmetic() {
        successful_sequence(0);
    }

    #[test]
    fn failed_first_counter_keeps_other_probes_without_a_sample_claim() {
        let calls = Cell::new(0);
        let (sample, other) = sample(
            || {
                calls.set(calls.get() + 1);
                Err(NativeError::InvalidInput("counter unavailable".into()))
            },
            || "RAM/disk result",
            || Duration::ZERO,
            |_| panic!("failed first counter must not start a sampling wait"),
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(other, "RAM/disk result");
        assert!(
            sample
                .percent
                .unwrap_err()
                .to_string()
                .contains("counter unavailable")
        );
        assert!(sample.elapsed.is_none());
    }

    #[test]
    fn failed_second_counter_has_no_successful_sample_duration() {
        let now = Cell::new(Duration::ZERO);
        let calls = Cell::new(0);
        let (sample, other) = sample(
            || {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Ok(CpuTimes {
                        idle: 0,
                        kernel: 0,
                        user: 0,
                    })
                } else {
                    Err(NativeError::InvalidInput(
                        "second counter unavailable".into(),
                    ))
                }
            },
            || "independent results",
            || now.get(),
            |duration| now.set(now.get() + duration),
        );
        assert_eq!(calls.get(), 2);
        assert_eq!(now.get(), Duration::from_millis(200));
        assert_eq!(other, "independent results");
        assert!(
            sample
                .percent
                .unwrap_err()
                .to_string()
                .contains("second counter")
        );
        assert!(sample.elapsed.is_none());
    }

    #[test]
    fn regressing_clock_is_an_error_not_a_fabricated_interval() {
        let observations = Cell::new(0);
        let (sample, other) = sample(
            || {
                Ok(CpuTimes {
                    idle: 0,
                    kernel: 0,
                    user: 0,
                })
            },
            || "other results",
            || {
                observations.set(observations.get() + 1);
                if observations.get() == 1 {
                    Duration::from_secs(1)
                } else {
                    Duration::ZERO
                }
            },
            |_| panic!("invalid clock must not start a wait"),
        );
        assert_eq!(other, "other results");
        assert!(
            sample
                .percent
                .unwrap_err()
                .to_string()
                .contains("clock moved backwards")
        );
        assert!(sample.elapsed.is_none());
    }
}
