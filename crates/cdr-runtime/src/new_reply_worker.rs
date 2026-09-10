//! Existing bot process: restart-safe new-input verification and normal-ack recovery.
mod reply;
mod scan;

use crate::{app_backend::AppServerTurnBackend, queue_runner::QueueCoordinator};
use cdr_store::new_reply::{self, CheckpointUpdate};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::watch,
    time::{MissedTickBehavior, interval},
};

pub(crate) async fn run(
    database: PathBuf,
    queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    http: Arc<twilight_http::Client>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut timer = interval(Duration::from_secs(1));
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            changed=shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return; },
            _=timer.tick() => {
                let db=database.clone();
                let batch=tokio::task::spawn_blocking(move||reconcile(&db)).await;
                match batch {
                    Ok(Ok(records)) => {
                        let has_work=!records.is_empty();
                        if has_work { queue.notify_delivery_ready(); }
                        for record in records {
                            match tokio::time::timeout(Duration::from_secs(5),reply::recover(&database,&http,&record)).await {
                                Ok(Ok(()))=>{},
                                Ok(Err(error))=>eprintln!("new_first_reply_recovery_hold job_id={} error={error}",record.identity.job_id),
                                Err(error)=>eprintln!("new_first_reply_recovery_timeout job_id={} outcome=unknown error={error}",record.identity.job_id),
                            }
                        }
                        if has_work { queue.notify_delivery_ready(); }
                    }
                    Ok(Err(error)) => eprintln!("new_first_reply_verification_error: {error}"),
                    Err(error) => eprintln!("new_first_reply_verification_task_error: {error}"),
                }
            }
        }
    }
}

pub(crate) fn reconcile(database: &std::path::Path) -> cdr_store::Result<Vec<new_reply::NewReply>> {
    let mut ready = Vec::new();
    for record in new_reply::pending(database, 8)? {
        let inspected = scan::inspect(&record);
        let (cursor, verified, error) = match inspected {
            Ok(scan) => (scan.checkpoint, scan.verified, String::new()),
            Err(error) => (record.scan.clone(), false, error),
        };
        if error != record.last_error && !error.is_empty() {
            eprintln!(
                "new_first_reply_verification_hold job_id={} error={error}",
                record.identity.job_id
            );
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
        if new_reply::checkpoint(
            database,
            &record,
            CheckpointUpdate {
                scan: &cursor,
                verified,
                error: &error,
                now,
            },
        )? && let Some(updated) = new_reply::get(database, &record.identity.job_id)?
        {
            ready.push(updated);
        }
    }
    Ok(ready)
}
