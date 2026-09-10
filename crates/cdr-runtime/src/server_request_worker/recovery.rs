use std::future::Future;
use std::pin::Pin;

use tokio::sync::watch;
use tokio::time::{Duration, Instant};

pub type BoxAttempt<'a, E> = Pin<Box<dyn Future<Output = Result<(), E>> + Send + 'a>>;
const RETRY_DELAYS: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
];

#[derive(Debug, Default)]
pub struct RetrySchedule {
    failures: usize,
    deadline: Option<Instant>,
}

impl RetrySchedule {
    pub fn schedule_failure(&mut self, now: Instant) {
        if self.deadline.is_some() {
            return;
        }
        let index = self.failures.min(RETRY_DELAYS.len() - 1);
        self.deadline = Some(now + RETRY_DELAYS[index]);
        self.failures = self.failures.saturating_add(1);
    }

    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub fn begin_retry(&mut self) {
        self.deadline = None;
    }

    pub fn clear(&mut self) {
        self.failures = 0;
        self.deadline = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("app-server generation changed: expected {expected}, current {actual}")]
pub struct GenerationMismatch {
    pub expected: u64,
    pub actual: u64,
}

pub fn validate_generation(expected: u64, actual: u64) -> Result<(), GenerationMismatch> {
    if expected == actual {
        Ok(())
    } else {
        Err(GenerationMismatch { expected, actual })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SnapshotError<E> {
    Fetch(E),
    Unstable { attempts: usize },
}

pub async fn cancel_on_shutdown<T>(
    shutdown: &mut watch::Receiver<bool>,
    future: impl Future<Output = T>,
) -> Option<T> {
    if *shutdown.borrow() {
        return None;
    }
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return None; }
            }
            output = &mut future => return Some(output),
        }
    }
}

pub async fn consistent_snapshot<T, E, G, F, Fut>(
    max_attempts: usize,
    mut generation: G,
    mut fetch: F,
) -> Result<(u64, T), SnapshotError<E>>
where
    G: FnMut() -> u64,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    for _ in 0..max_attempts {
        let before = generation();
        let snapshot = fetch().await.map_err(SnapshotError::Fetch)?;
        if generation() == before {
            return Ok((before, snapshot));
        }
    }
    Err(SnapshotError::Unstable {
        attempts: max_attempts,
    })
}

pub async fn attempt_all<S, I, E, F>(
    state: &mut S,
    items: impl IntoIterator<Item = I>,
    mut attempt: F,
) -> Result<(), E>
where
    F: for<'a> FnMut(&'a mut S, I) -> BoxAttempt<'a, E>,
{
    let mut first_error = None;
    for item in items {
        if let Err(error) = attempt(state, item).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}
