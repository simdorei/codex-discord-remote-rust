use std::{collections::VecDeque, mem, time::Duration};

use tokio::{
    task::JoinHandle,
    time::{Instant, timeout_at},
};

use super::GatewayShutdownError;

pub(super) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const ABORT_JOIN_RESERVE: Duration = Duration::from_secs(1);

pub(super) struct GatewayTask {
    shard: u32,
    handle: JoinHandle<()>,
}

impl GatewayTask {
    pub(super) fn new(shard: u32, handle: JoinHandle<()>) -> Self {
        Self { shard, handle }
    }

    pub(super) fn abort(&self) {
        self.handle.abort();
    }
}

pub struct GatewayShutdownReport {
    trigger: Option<Result<(), GatewayShutdownError>>,
    cleanup: Result<(), GatewayShutdownError>,
}

impl GatewayShutdownReport {
    #[doc(hidden)]
    #[must_use]
    pub fn from_parts(
        trigger: Option<Result<(), GatewayShutdownError>>,
        cleanup: Result<(), GatewayShutdownError>,
    ) -> Self {
        Self { trigger, cleanup }
    }

    pub fn into_parts(
        self,
    ) -> (
        Option<Result<(), GatewayShutdownError>>,
        Result<(), GatewayShutdownError>,
    ) {
        (self.trigger, self.cleanup)
    }
}

#[cfg(test)]
pub(super) async fn join_gateway_tasks(
    tasks: &mut Vec<JoinHandle<()>>,
    timeout: Duration,
) -> Result<(), GatewayShutdownError> {
    let mut identified = mem::take(tasks)
        .into_iter()
        .map(|handle| GatewayTask::new(0, handle))
        .collect::<Vec<_>>();
    let report =
        join_gateway_tasks_for_cause(&mut identified, Instant::now() + timeout, None).await;
    debug_assert!(identified.is_empty());
    report.cleanup
}

pub(super) async fn join_gateway_tasks_until(
    tasks: &mut Vec<GatewayTask>,
    deadline: Instant,
) -> Result<(), GatewayShutdownError> {
    join_gateway_tasks_for_cause(tasks, deadline, None)
        .await
        .cleanup
}

pub(super) async fn join_gateway_tasks_for_cause(
    tasks: &mut Vec<GatewayTask>,
    deadline: Instant,
    trigger: Option<u32>,
) -> GatewayShutdownReport {
    let mut pending = VecDeque::from(mem::take(tasks));
    let trigger = if let Some(shard) = trigger {
        match take_shard(&mut pending, shard) {
            Some(mut task) => Some(join_trigger(&mut task, deadline).await),
            None => None,
        }
    } else {
        None
    };
    let cleanup = join_remaining(pending, deadline).await;
    GatewayShutdownReport { trigger, cleanup }
}

fn take_shard(tasks: &mut VecDeque<GatewayTask>, shard: u32) -> Option<GatewayTask> {
    let position = tasks.iter().position(|task| task.shard == shard)?;
    tasks.remove(position)
}

async fn join_trigger(
    task: &mut GatewayTask,
    deadline: Instant,
) -> Result<(), GatewayShutdownError> {
    let result = match timeout_at(deadline, &mut task.handle).await {
        Ok(result) => result.map_err(Into::into),
        Err(_) => terminate_on_shutdown_timeout(),
    };
    if let Err(error) = &result {
        eprintln!(
            "primary_gateway_shutdown_error shard={} error={error}",
            task.shard
        );
    }
    result
}

async fn join_remaining(
    mut pending: VecDeque<GatewayTask>,
    deadline: Instant,
) -> Result<(), GatewayShutdownError> {
    let drain_deadline = deadline
        .checked_sub(ABORT_JOIN_RESERVE)
        .unwrap_or(deadline)
        .max(Instant::now());
    let mut first_join_error = None;
    while let Some(mut task) = pending.pop_front() {
        if let Ok(result) = timeout_at(drain_deadline, &mut task.handle).await {
            record_join(result, false, &mut first_join_error);
        } else {
            pending.push_front(task);
            break;
        }
    }
    if pending.is_empty() {
        return first_join_error.map_or(Ok(()), |error| Err(GatewayShutdownError::Join(error)));
    }

    for task in &pending {
        task.abort();
    }
    while let Some(mut task) = pending.pop_front() {
        match timeout_at(deadline, &mut task.handle).await {
            Ok(result) => record_join(result, true, &mut first_join_error),
            Err(_) => terminate_on_shutdown_timeout(),
        }
    }
    first_join_error.map_or(Err(GatewayShutdownError::Timeout), |error| {
        Err(GatewayShutdownError::Join(error))
    })
}

fn record_join(
    result: Result<(), tokio::task::JoinError>,
    forced: bool,
    first_error: &mut Option<tokio::task::JoinError>,
) {
    if let Err(error) = result
        && !(forced && error.is_cancelled())
        && first_error.is_none()
    {
        *first_error = Some(error);
    }
}

#[cold]
fn terminate_on_shutdown_timeout() -> ! {
    eprintln!("fatal_gateway_abort_join_timeout");
    #[cfg(not(test))]
    std::process::abort();
    #[cfg(test)]
    panic!("fatal gateway shutdown timeout");
}
