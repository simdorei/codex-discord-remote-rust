use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::{Instant, sleep_until};

use super::ResidentAppServer;
use crate::AppServerError;

const INITIAL_RETRY: Duration = Duration::from_millis(250);
const MAX_RETRY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct RestartBackoff {
    failures: u32,
}

impl RestartBackoff {
    pub(super) fn next_delay(&mut self) -> Duration {
        let shift = self.failures.min(5);
        self.failures = self.failures.saturating_add(1);
        INITIAL_RETRY
            .checked_mul(1_u32 << shift)
            .unwrap_or(MAX_RETRY)
            .min(MAX_RETRY)
    }

    pub(super) fn reset(&mut self) {
        self.failures = 0;
    }
}

impl ResidentAppServer {
    pub async fn run_restart_supervisor(self: Arc<Self>, shutdown: watch::Receiver<bool>) {
        let restart = self.state.subscribe_restart_pending();
        run_restart_supervisor_using(restart, shutdown, move |generation| {
            let server = Arc::clone(&self);
            async move { server.restart_generation_if_quiescent(generation).await }
        })
        .await;
    }
}

pub(super) async fn run_restart_supervisor_using<Attempt, AttemptFuture>(
    mut restart: watch::Receiver<Option<u64>>,
    mut shutdown: watch::Receiver<bool>,
    mut attempt: Attempt,
) where
    Attempt: FnMut(u64) -> AttemptFuture,
    AttemptFuture: Future<Output = Result<bool, AppServerError>>,
{
    let mut backoff = RestartBackoff::default();
    let mut retry_generation = None;
    let mut settled_through = None;
    loop {
        if shutdown_requested(&shutdown) {
            return;
        }
        let requested = *restart.borrow_and_update();
        let Some(generation) = requested else {
            reset_retry(&mut backoff, &mut retry_generation);
            if wait_for_change(&mut restart, &mut shutdown).await {
                return;
            }
            continue;
        };
        if settled_through.is_some_and(|settled| generation <= settled) {
            reset_retry(&mut backoff, &mut retry_generation);
            if wait_for_change(&mut restart, &mut shutdown).await {
                return;
            }
            continue;
        }
        if retry_generation != Some(generation) {
            reset_retry(&mut backoff, &mut retry_generation);
            retry_generation = Some(generation);
        }
        let settled = match attempt(generation).await {
            Ok(settled) => settled,
            Err(error) => {
                eprintln!(
                    "app_server_restart_pending_retry_failed generation={generation} error={error}"
                );
                false
            }
        };
        if shutdown_requested(&shutdown) {
            return;
        }
        if settled {
            settled_through =
                Some(settled_through.map_or(generation, |old: u64| old.max(generation)));
            reset_retry(&mut backoff, &mut retry_generation);
            continue;
        }
        let retry_at = Instant::now() + backoff.next_delay();
        match wait_for_retry(&mut restart, &mut shutdown, generation, retry_at).await {
            RetryWait::Stop => return,
            RetryWait::Changed | RetryWait::Deadline => {}
        }
    }
}

fn reset_retry(backoff: &mut RestartBackoff, generation: &mut Option<u64>) {
    backoff.reset();
    *generation = None;
}

fn shutdown_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn wait_for_change(
    restart: &mut watch::Receiver<Option<u64>>,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    loop {
        tokio::select! {
            changed = restart.changed() => return changed.is_err(),
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown_requested(shutdown) {
                    return true;
                }
            }
        }
    }
}

enum RetryWait {
    Changed,
    Deadline,
    Stop,
}

async fn wait_for_retry(
    restart: &mut watch::Receiver<Option<u64>>,
    shutdown: &mut watch::Receiver<bool>,
    generation: u64,
    deadline: Instant,
) -> RetryWait {
    let sleep = sleep_until(deadline);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return RetryWait::Deadline,
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown_requested(shutdown) {
                    return RetryWait::Stop;
                }
            }
            changed = restart.changed() => {
                if changed.is_err() {
                    return RetryWait::Stop;
                }
                if *restart.borrow_and_update() != Some(generation) {
                    return RetryWait::Changed;
                }
            }
        }
    }
}
