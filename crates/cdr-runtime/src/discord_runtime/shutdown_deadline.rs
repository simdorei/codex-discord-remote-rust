use std::{future::Future, time::Duration};

use tokio::time::{Instant, timeout_at};

pub(super) const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const NON_HEARTBEAT_RESERVE: Duration = Duration::from_secs(15);
pub(super) const HEARTBEAT_JOIN_RESERVE: Duration = Duration::from_secs(1);

pub(super) async fn complete_before_shutdown_deadline<F>(
    deadline: Instant,
    component: &'static str,
    future: F,
) -> F::Output
where
    F: Future,
{
    tokio::pin!(future);
    let Ok(output) = timeout_at(deadline, &mut future).await else {
        terminate_on_shutdown_timeout(component);
    };
    output
}

#[cold]
pub(super) fn terminate_on_shutdown_timeout(component: &str) -> ! {
    eprintln!("fatal_runtime_shutdown_timeout component={component}");
    #[cfg(not(test))]
    std::process::abort();
    #[cfg(test)]
    {
        if std::env::var_os("CDR_TEST_REAL_SHUTDOWN_ABORT").is_some() {
            std::process::abort();
        }
        panic!("fatal runtime shutdown timeout: {component}");
    }
}

#[cfg(test)]
#[path = "shutdown_deadline_tests.rs"]
mod tests;
