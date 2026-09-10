use std::time::Duration;

use tokio::sync::watch;
use tokio::time::{Instant, sleep};

use crate::session_mirror::{SessionMirrorError, SessionMirrorPoll, SessionMirrorSender};
use crate::session_mirror_worker::SessionMirrorWorker;

const NORMAL_POLL_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
const REPEAT_REPORT_INTERVAL: Duration = Duration::from_mins(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMirrorFailureReport {
    pub count: u64,
    pub error: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMirrorRetryDecision {
    pub retry_after: Duration,
    pub report: Option<SessionMirrorFailureReport>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionMirrorRetryState {
    consecutive_failures: u64,
    last_error: Option<String>,
    next_report_at: Option<Duration>,
}

impl SessionMirrorRetryState {
    #[must_use]
    pub fn on_failure(&mut self, now: Duration, error: &str) -> SessionMirrorRetryDecision {
        let changed = self.last_error.as_deref() != Some(error);
        let report = if changed {
            self.consecutive_failures = 1;
            self.last_error = Some(error.to_owned());
            self.next_report_at = Some(report_deadline(now));
            Some(self.failure_report(error))
        } else {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            self.repeat_report(now, error)
        };
        SessionMirrorRetryDecision {
            retry_after: retry_delay(self.consecutive_failures),
            report,
        }
    }

    #[must_use]
    pub fn on_success(&mut self) -> Duration {
        *self = Self::default();
        NORMAL_POLL_DELAY
    }

    fn repeat_report(&mut self, now: Duration, error: &str) -> Option<SessionMirrorFailureReport> {
        if self.next_report_at.is_some_and(|deadline| now < deadline) {
            return None;
        }
        self.next_report_at = Some(report_deadline(now));
        Some(self.failure_report(error))
    }

    fn failure_report(&self, error: &str) -> SessionMirrorFailureReport {
        SessionMirrorFailureReport {
            count: self.consecutive_failures,
            error: error.to_owned(),
        }
    }
}

pub async fn run_session_mirror_worker<S: SessionMirrorSender + 'static>(
    worker: SessionMirrorWorker<S>,
    mut shutdown: watch::Receiver<bool>,
) {
    let started_at = Instant::now();
    let mut retry_state = SessionMirrorRetryState::default();
    let mut next_poll_after = Duration::ZERO;
    loop {
        if wait_or_shutdown(next_poll_after, &mut shutdown).await {
            return;
        }
        let Some(result) = poll_or_shutdown(&worker, &mut shutdown).await else {
            return;
        };
        next_poll_after = match result {
            Ok(result) => {
                if result.sent > 0 {
                    eprintln!("session_mirror_poll: {result:?}");
                }
                retry_state.on_success()
            }
            Err(error) => {
                let decision = retry_state.on_failure(started_at.elapsed(), &error.to_string());
                if let Some(report) = decision.report {
                    eprintln!(
                        "session_mirror_error count={} error={}",
                        report.count, report.error
                    );
                }
                decision.retry_after
            }
        };
    }
}

async fn wait_or_shutdown(delay: Duration, shutdown: &mut watch::Receiver<bool>) -> bool {
    if *shutdown.borrow() {
        return true;
    }
    let timer = sleep(delay);
    tokio::pin!(timer);
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return true; }
            }
            () = &mut timer => return false,
        }
    }
}

async fn poll_or_shutdown<S: SessionMirrorSender>(
    worker: &SessionMirrorWorker<S>,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<Result<SessionMirrorPoll, SessionMirrorError>> {
    if *shutdown.borrow() {
        return None;
    }
    let poll = worker.poll_once();
    tokio::pin!(poll);
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return None; }
            }
            result = &mut poll => return Some(result),
        }
    }
}

fn retry_delay(consecutive_failures: u64) -> Duration {
    let exponent = u32::try_from(consecutive_failures.saturating_sub(1).min(5)).unwrap_or(5);
    Duration::from_secs((1_u64 << exponent).min(MAX_RETRY_DELAY.as_secs()))
}

fn report_deadline(now: Duration) -> Duration {
    now.checked_add(REPEAT_REPORT_INTERVAL)
        .unwrap_or(Duration::MAX)
}
