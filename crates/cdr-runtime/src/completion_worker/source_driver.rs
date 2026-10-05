//! Production indexed intake. Public broadcast events are wakeups, not sequence.
#[cfg(test)]
mod review_tests;
use super::observation_gap as gap;
use super::scheduler::lanes::{EVENT_BYTES, Envelope};
use super::{Arc, CompletionWorker};
use cdr_app_server::{ResidentNotificationEvent, observation::ObservationWindow};
use cdr_store::observation_gap::Scope;
use std::time::Duration;
use tokio::sync::{Semaphore, broadcast, mpsc, watch};

pub(super) async fn observe(
    worker: Arc<CompletionWorker>,
    mut receiver: broadcast::Receiver<ResidentNotificationEvent>,
    sender: mpsc::Sender<Envelope>,
    budget: Arc<Semaphore>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut current: Option<Scope> = None;
    let mut forwarded = 0;
    loop {
        tokio::select! {
            biased;
            changed=shutdown.changed()=>{if changed.is_err() || *shutdown.borrow(){return;}},
            _=tick.tick()=>{},
            incoming=receiver.recv()=>match incoming {
                Ok(ResidentNotificationEvent::Gap{generation,..})=>worker.server.mark_source_observation_gap(generation),
                Ok(event@ResidentNotificationEvent::Notification{generation,..}) if generation!=worker.server.generation()=>{
                    // Preserve exact late terminal journaling from the old forwarding path.
                    // It cannot certify the current stream or become a new occurrence.
                    CompletionWorker::report(worker.observe_terminal(&event));
                    send(&worker,&sender,&budget,event);
                },
                Ok(_)=>{},
                Err(broadcast::error::RecvError::Lagged(_))=>worker.server.mark_source_observation_gap(worker.server.generation()),
                Err(broadcast::error::RecvError::Closed)=>return,
            }
        }
        let scope = match gap::scope(&worker) {
            Ok(s) => s,
            Err(e) => {
                CompletionWorker::report(Err(e));
                continue;
            }
        };
        if current.as_ref() != Some(&scope) {
            if let Err(error) = activate(&worker, &scope).await {
                worker.server.mark_idle_observation_gap();
                eprintln!("observation_stream_open_error error={error}");
                continue;
            }
            current = Some(scope.clone());
            forwarded = 0;
        }
        let page =
            match worker
                .server
                .observation_window(worker.server.generation(), forwarded, None)
            {
                Ok(p) => p,
                Err(e) => {
                    worker.server.mark_idle_observation_gap();
                    eprintln!("observation_window_error error={e}");
                    continue;
                }
            };
        if !process_page(&worker, &scope, &page, &sender, &budget).await {
            continue;
        }
        forwarded = page.scanned_through;
        tokio::task::yield_now().await;
    }
}
async fn activate(
    worker: &Arc<CompletionWorker>,
    scope: &Scope,
) -> Result<(), super::CompletionWorkerError> {
    let path = worker.queue.db_path().to_owned();
    let scope = scope.clone();
    tokio::task::spawn_blocking(move || cdr_store::observation_gap::activate(&path, &scope))
        .await
        .map_err(|error| {
            super::CompletionWorkerError::Held(format!(
                "observation activation worker failed: {error}"
            ))
        })??;
    Ok(())
}
async fn process_page(
    worker: &Arc<CompletionWorker>,
    scope: &Scope,
    page: &ObservationWindow,
    sender: &mpsc::Sender<Envelope>,
    budget: &Arc<Semaphore>,
) -> bool {
    let owned = Arc::clone(worker);
    let owned_scope = scope.clone();
    let upper = page.source_upper;
    let saved = tokio::task::spawn_blocking(move || {
        cdr_store::observation_gap::discover(
            owned.queue.db_path(),
            &owned_scope,
            i64::try_from(upper)
                .map_err(|_| cdr_store::StoreError::Integrity("source sequence range".into()))?,
        )
    })
    .await;
    if !matches!(saved, Ok(Ok(()))) {
        worker.server.mark_idle_observation_gap();
        eprintln!("observation_intent_store_error result={saved:?}");
        // The pre-intake unsealed anchor retains uncertainty even if this
        // narrower intent cannot be saved. Do not grant a verified prefix.
    }
    if page.first_available
        > page
            .events
            .first()
            .map_or(page.scanned_through.saturating_add(1), |e| e.sequence)
    {
        worker.server.mark_source_observation_gap(page.generation);
    }
    for original in &page.events {
        let Some(notification) = original.notification.clone() else {
            worker.server.mark_source_observation_gap(page.generation);
            continue;
        };
        if notification.method == "turn/completed"
            && let Ok(completion) =
                cdr_app_server::outcomes::parse_turn_completion(&notification.params, false)
        {
            worker
                .terminal_fence
                .stop(page.generation, &completion.thread_id, &completion.turn_id);
        }
        let owned = Arc::clone(worker);
        let scope = scope.clone();
        let observed = notification.clone();
        let seq = original.sequence;
        match tokio::task::spawn_blocking(move || {
            gap::certify_event(&owned, &scope, seq, &observed)
        })
        .await
        {
            Ok(result) => {
                if result.is_err() {
                    worker.server.mark_source_observation_gap(page.generation);
                }
                CompletionWorker::report(result);
            }
            Err(error) => {
                worker.server.mark_source_observation_gap(page.generation);
                eprintln!("observation_journal_join_error error={error}");
            }
        }
        // No required journal failure advances the new verified source ledger.
        // Existing state processing may still complete its already-owned work.
        let event = ResidentNotificationEvent::Notification {
            generation: page.generation,
            notification,
        };
        send(worker, sender, budget, event);
    }
    true
}
fn send(
    worker: &CompletionWorker,
    sender: &mpsc::Sender<Envelope>,
    budget: &Arc<Semaphore>,
    event: ResidentNotificationEvent,
) {
    if matches!(&event,ResidentNotificationEvent::Notification{notification,..}
        if cdr_app_server::extract_thread_id(&notification.params).is_none())
    {
        return;
    }
    let Some(charged) = Envelope::charge(event, budget) else {
        worker
            .server
            .mark_source_observation_gap(worker.server.generation());
        eprintln!("completion_event_budget_gap bytes={EVENT_BYTES}");
        return;
    };
    if let Err(error) = sender.try_send(charged) {
        worker
            .server
            .mark_source_observation_gap(worker.server.generation());
        eprintln!("completion_processing_queue_gap error={error}; original ranges retained");
    }
}
