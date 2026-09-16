//! Bounded, cancellable recovery scheduling. All mutations stay under queue custody.
use super::ReserveAutoController;
use crate::{app_backend::AppServerTurnBackend, queue_runner::QueueCoordinator};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::watch,
    time::{Instant, timeout_at},
};

/// Returns true only when shutdown was requested. Cursor tracks attempted targets,
/// including busy/failed ones, so one locked target cannot starve later episodes.
pub(crate) async fn run_recovery_cycle(
    controller: &ReserveAutoController,
    queue: &Arc<QueueCoordinator<AppServerTurnBackend>>,
    shutdown: &mut watch::Receiver<bool>,
    cursor: &mut Option<String>,
    budget: Duration,
) -> bool {
    if *shutdown.borrow() {
        return true;
    }
    let deadline = Instant::now() + budget;
    let mut targets = controller.poll_recovery_after(cursor.as_deref());
    if targets.is_empty() && cursor.take().is_some() {
        targets = controller.poll_recovery_after(None);
    }
    for target in targets {
        if *shutdown.borrow() {
            return true;
        }
        if Instant::now() >= deadline {
            break;
        }
        *cursor = Some(target.clone());
        let result = tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return true; }
                continue;
            }
            result = timeout_at(deadline, queue.recover_reserve_target(controller, &target)) => result,
        };
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                eprintln!("reserve_auto_recovery_deferred thread_id={target} error={error}");
            }
            Err(_) => {
                // If bytes were dispatched, entering/restoring or Starting was
                // committed first. Never clear it, replay it, or read the latest
                // owner's revision after dropping the lock to mark it unknown.
                eprintln!(
                    "reserve_auto_recovery_budget_exceeded thread_id={target}; dispatched intent preserved, no retry issued"
                );
                break;
            }
        }
    }
    false
}
