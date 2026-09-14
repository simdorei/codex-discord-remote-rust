use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{MissedTickBehavior, interval};

use super::{CompletionWorker, CompletionWorkerError};
use cdr_app_server::ResidentNotificationEvent;
use tokio::sync::{broadcast, mpsc, watch};

pub(super) async fn run(
    worker: Arc<CompletionWorker>,
    receiver: broadcast::Receiver<ResidentNotificationEvent>,
    shutdown: watch::Receiver<bool>,
) {
    if *shutdown.borrow() {
        return;
    }
    let (sender, pending) = mpsc::channel(128);
    let mut tasks = tokio::task::JoinSet::new();
    let processing = Arc::clone(&worker);
    tasks.spawn(async move { process(&processing, pending).await });
    tasks.spawn(super::idle_release::run(
        Arc::clone(&worker),
        shutdown.clone(),
    ));
    let work = async {
        tokio::select! {
            () = observe(Arc::clone(&worker), receiver, sender, shutdown) => {},
            result = tasks.join_next() => { eprintln!("completion_processor_stopped result={result:?}"); },
        }
    };
    with_heartbeat(work, || worker.send_typing(), Duration::from_secs(6)).await;
    tasks.abort_all();
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result
            && !error.is_cancelled()
        {
            eprintln!("completion_processor_join_error error={error}");
        }
    }
}

async fn observe(
    worker: Arc<CompletionWorker>,
    mut receiver: broadcast::Receiver<ResidentNotificationEvent>,
    sender: mpsc::Sender<ResidentNotificationEvent>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { return; } },
            incoming = receiver.recv() => {
                match incoming {
                    Ok(event) => {
                        if let ResidentNotificationEvent::Notification{generation,notification}=&event {
                            if notification.method=="turn/completed"
                                && let Ok(completion)=cdr_app_server::outcomes::parse_turn_completion(&notification.params,false) {
                                worker.terminal_fence.stop(*generation,&completion.thread_id,&completion.turn_id);
                            }
                            if notification.method=="item/completed"
                                && notification.params.get("item").is_some_and(cdr_app_server::async_questions::is_async_message) {
                                let db=worker.queue.db_path().to_owned();
                                let runtime=worker.server.instance_id().to_owned();
                                let generation=*generation;
                                let params=notification.params.clone();
                                match tokio::task::spawn_blocking(move||crate::async_question_ui::observe(&db,&runtime,generation,&params)).await {
                                    Ok(result)=>{
                                        if result.is_err() { worker.server.mark_idle_observation_gap(); }
                                        CompletionWorker::report(result);
                                    },
                                    Err(error)=>{
                                        worker.server.mark_idle_observation_gap();
                                        eprintln!("async_question_journal_error error={error}");
                                    },
                                }
                            }
                            let durable_completion_event = notification.method == "turn/completed"
                                || (notification.method == "item/completed"
                                    && cdr_app_server::outcomes::extract_completed_final_answer(
                                        &notification.params,
                                    )
                                    .is_some());
                            if durable_completion_event {
                                // One bounded DB operation, off the async scheduler. Final text and
                                // terminal metadata are durable before completion processing.
                                let journal_worker=Arc::clone(&worker);
                                let observed=event.clone();
                                match tokio::task::spawn_blocking(move||journal_worker.observe_terminal(&observed)).await {
                                    Ok(result)=>{
                                        if result.is_err() { worker.server.mark_idle_observation_gap(); }
                                        CompletionWorker::report(result);
                                    },
                                    Err(error)=>{
                                        worker.server.mark_idle_observation_gap();
                                        eprintln!("completion_journal_task_error error={error}");
                                    },
                                }
                            }
                            worker.server.confirm_idle_observation(*generation, notification);
                        } else {
                            worker.server.mark_idle_observation_gap();
                        }
                        if let Err(error) = sender.try_send(event) {
                            worker.server.mark_idle_observation_gap();
                            eprintln!("completion_processing_queue_gap error={error}; durable terminals retained; readonly reconciliation required");
                        }
                    },
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        worker.server.mark_idle_observation_gap();
                        eprintln!("app_server_completion_gap skipped={skipped}; readonly reconciliation required");
                    },
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    }
}

pub(super) async fn process(
    worker: &CompletionWorker,
    mut pending: mpsc::Receiver<ResidentNotificationEvent>,
) {
    CompletionWorker::report(worker.recover_observed().await);
    CompletionWorker::report(worker.recover_goal_progress().await);
    CompletionWorker::report(worker.recover().await);
    CompletionWorker::report(worker.deliver_questions().await);
    let mut retry = interval(Duration::from_secs(30));
    retry.set_missed_tick_behavior(MissedTickBehavior::Delay);
    retry.tick().await;
    loop {
        tokio::select! {
            () = worker.queue.wait_for_delivery_ready() => {
                CompletionWorker::report(worker.deliver_questions().await);
                CompletionWorker::report(worker.deliver_pending_commentary().await);
                CompletionWorker::report(worker.recover_goal_progress().await);
                CompletionWorker::report(worker.deliver_pending().await);
            },
            event = pending.recv() => match event {
                Some(event) => CompletionWorker::report(worker.handle(event).await),
                None => return,
            },
            _ = retry.tick() => {
                CompletionWorker::report(worker.deliver_questions().await);
                CompletionWorker::report(worker.recover_observed().await);
                CompletionWorker::report(worker.recover_goal_progress().await);
                CompletionWorker::report(worker.recover().await);
            }
        }
    }
}

// Scoped futures: no detached heartbeat can outlive its owning worker.
// Poll the work independently even while a Discord typing request is pending.
pub(super) async fn with_heartbeat<W, H, HF>(work: W, mut heartbeat: H, period: Duration)
where
    W: Future<Output = ()>,
    H: FnMut() -> HF,
    HF: Future<Output = Result<(), CompletionWorkerError>>,
{
    let ticks = async {
        let mut timer = interval(period);
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            CompletionWorker::report(heartbeat().await);
        }
    };
    tokio::select! {
        biased;
        () = work => {},
        () = ticks => {},
    }
}

#[cfg(test)]
#[path = "async_question_driver_tests.rs"]
mod async_question_tests;

#[cfg(test)]
#[path = "async_question_goal_tests.rs"]
mod async_question_goal_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test(start_paused = true)]
    async fn slow_work_does_not_block_six_second_ticks_and_finish_cancels_them() {
        let count = Arc::new(AtomicUsize::new(0));
        let ticks = Arc::clone(&count);
        with_heartbeat(
            tokio::time::sleep(Duration::from_secs(30)),
            move || {
                ticks.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            },
            Duration::from_secs(6),
        )
        .await;
        assert_eq!(count.load(Ordering::SeqCst), 5);
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(count.load(Ordering::SeqCst), 5);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_typing_request_does_not_delay_work_completion() {
        let started = tokio::time::Instant::now();
        with_heartbeat(
            tokio::time::sleep(Duration::from_secs(2)),
            || async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(())
            },
            Duration::from_secs(6),
        )
        .await;
        assert_eq!(started.elapsed(), Duration::from_secs(2));
    }
}
