//! Bounded source reconciliation: local journals only, never generic handler replay.
use super::{Arc, CompletionWorker, CompletionWorkerError, Duration, watch};
use cdr_app_server::{Notification, ResidentNotificationEvent, observation::ObservationWindow};
use cdr_store::async_question::QuestionBody;
use cdr_store::observation_gap::{self as store, Effect, Scope};
#[cfg(test)]
mod tests;
use std::sync::atomic::{AtomicBool, Ordering};

struct CancelPage(Arc<AtomicBool>);
impl Drop for CancelPage {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub(super) fn scope(worker: &CompletionWorker) -> Result<Scope, CompletionWorkerError> {
    Ok(Scope {
        owner_id: worker.server.instance_id().to_owned(),
        generation: i64::try_from(worker.server.generation())
            .map_err(|_| crate::queue_runner::QueueRunnerError::IntegerRange)?,
    })
}
fn sequence(value: u64) -> Result<i64, CompletionWorkerError> {
    i64::try_from(value).map_err(|_| crate::queue_runner::QueueRunnerError::IntegerRange.into())
}
pub(super) fn journal_effects(
    worker: &CompletionWorker,
    scope: &Scope,
    n: &Notification,
) -> Result<Vec<Effect>, CompletionWorkerError> {
    let generation = u64::try_from(scope.generation)
        .map_err(|_| crate::queue_runner::QueueRunnerError::IntegerRange)?;
    let event = ResidentNotificationEvent::Notification {
        generation,
        notification: n.clone(),
    };
    worker.observe_terminal(&event)?;
    match n.method.as_str() {
        "turn/completed" => {
            let c = cdr_app_server::outcomes::parse_turn_completion(&n.params, false)?;
            Ok(vec![Effect::Terminal {
                thread: c.thread_id.clone(),
                turn: c.turn_id.clone(),
                payload: cdr_app_server::outcomes::completion_journal_payload(&c).to_string(),
            }])
        }
        "turn/started" => Ok(
            match (
                cdr_app_server::extract_thread_id(&n.params),
                cdr_app_server::extract_turn_id(&n.params),
            ) {
                (Some(thread), Some(turn)) => vec![Effect::Started { thread, turn }],
                _ => vec![Effect::Unconfirmed],
            },
        ),
        "item/completed" => {
            if n.params
                .get("item")
                .is_some_and(cdr_app_server::async_questions::is_async_message)
            {
                crate::async_question_ui::observe(
                    worker.queue.db_path(),
                    &scope.owner_id,
                    generation,
                    &n.params,
                )?;
                return question_effects(&n.params);
            }
            if let Some(answer) =
                cdr_app_server::outcomes::extract_completed_final_answer(&n.params)
            {
                return Ok(vec![Effect::Final {
                    thread: answer.thread_id,
                    turn: answer.turn_id,
                    content: answer.text,
                }]);
            }
            Ok(vec![if worker.commentary_enabled {
                Effect::Unconfirmed
            } else {
                Effect::NoRequiredStore
            }])
        }
        // A non-active Goal update may have a missing ownership effect. Do not
        // guess its turn from current history or manufacture a successor.
        "thread/goal/updated" => Ok(vec![Effect::Unconfirmed]),
        _ => Ok(vec![
            if worker.commentary_enabled && n.method.starts_with("item/") {
                Effect::Unconfirmed
            } else {
                Effect::NoRequiredStore
            },
        ]),
    }
}
fn question_effects(params: &serde_json::Value) -> Result<Vec<Effect>, CompletionWorkerError> {
    let Some(mut parsed) =
        cdr_app_server::async_questions::parse(params).map_err(CompletionWorkerError::Delivery)?
    else {
        return Ok(vec![Effect::Unconfirmed]);
    };
    let source_text = if parsed.questions.is_empty() {
        parsed
            .questions
            .push(cdr_app_server::async_questions::AsyncQuestion {
                title: parsed.text.clone(),
                options: vec![],
            });
        String::new()
    } else {
        parsed.text.clone()
    };
    if parsed.questions.len() > store::PAGE_SIZE {
        return Ok(vec![Effect::Unconfirmed]);
    }
    parsed
        .questions
        .into_iter()
        .enumerate()
        .map(|(index, q)| {
            let body = QuestionBody {
                index,
                source_text: source_text.clone(),
                title: q.title,
                options: q.options,
            };
            Ok(Effect::Question {
                id: cdr_store::async_question::occurrence_id(
                    &parsed.thread_id,
                    &parsed.turn_id,
                    &parsed.item_id,
                    index,
                )?,
                thread: parsed.thread_id.clone(),
                turn: parsed.turn_id.clone(),
                item: parsed.item_id.clone(),
                body: serde_json::to_string(&body)
                    .map_err(|e| CompletionWorkerError::Delivery(e.to_string()))?,
            })
        })
        .collect()
}
pub(super) fn certify_event(
    worker: &CompletionWorker,
    scope: &Scope,
    seq: u64,
    n: &Notification,
) -> Result<(), CompletionWorkerError> {
    if scope.owner_id != worker.server.instance_id()
        || sequence(worker.server.generation())? != scope.generation
    {
        return Err(CompletionWorkerError::Held(
            "original observation scope changed".into(),
        ));
    }
    let effects = journal_effects(worker, scope, n)?;
    if sequence(worker.server.generation())? != scope.generation {
        return Err(CompletionWorkerError::Held(
            "observation generation changed during journal".into(),
        ));
    }
    let _ = store::certify(worker.queue.db_path(), scope, sequence(seq)?, &effects)?;
    Ok(())
}
pub(super) fn discover(
    worker: &CompletionWorker,
    scope: &Scope,
    page: &ObservationWindow,
) -> Result<(), CompletionWorkerError> {
    if page.owner_id != scope.owner_id || sequence(page.generation)? != scope.generation {
        return Err(CompletionWorkerError::Held(
            "different original observation stream".into(),
        ));
    }
    store::discover(worker.queue.db_path(), scope, sequence(page.source_upper)?)?;
    Ok(())
}
fn reconcile(
    worker: &CompletionWorker,
    scope: &Scope,
    cancelled: &AtomicBool,
) -> Result<(), CompletionWorkerError> {
    if cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let generation = u64::try_from(scope.generation)
        .map_err(|_| crate::queue_runner::QueueRunnerError::IntegerRange)?;
    let head = worker.server.observation_window(generation, 0, Some(0))?;
    discover(worker, scope, &head)?;
    if let Some(gap) = store::next(worker.queue.db_path(), scope)? {
        let page = worker.server.observation_window(
            generation,
            u64::try_from(gap.cursor)
                .map_err(|_| crate::queue_runner::QueueRunnerError::IntegerRange)?,
            Some(
                u64::try_from(gap.last)
                    .map_err(|_| crate::queue_runner::QueueRunnerError::IntegerRange)?,
            ),
        )?;
        for original in &page.events {
            if cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            if gap.contains_verified(sequence(original.sequence)?) {
                continue;
            }
            if let Some(n) = &original.notification {
                CompletionWorker::report(certify_event(worker, scope, original.sequence, n));
            }
        }
        // A proof writer may have changed this exact revision. Retry the next
        // bounded pass rather than acknowledging a stale claim.
        if !cancelled.load(Ordering::Acquire)
            && worker.server.generation() == generation
            && page.scanned_through > u64::try_from(gap.cursor).unwrap_or(u64::MAX)
        {
            let _ = store::finish_page(
                worker.queue.db_path(),
                &gap,
                sequence(page.scanned_through)?,
            )?;
        }
    }
    if cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let latest = worker.server.observation_window(generation, 0, Some(0))?;
    if store::scope_verified(
        worker.queue.db_path(),
        scope,
        sequence(latest.source_upper)?,
    )? {
        let _ = worker
            .server
            .reconcile_idle_observation_prefix(generation, latest.source_upper)?;
    }
    Ok(())
}
pub(super) async fn run(worker: Arc<CompletionWorker>, mut shutdown: watch::Receiver<bool>) {
    if !worker.server.observation_tracking_enabled() {
        return;
    }
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut current = None;
    loop {
        tokio::select! {
            biased;
            changed=shutdown.changed()=>{if changed.is_err() || *shutdown.borrow(){return;}},
            _=interval.tick()=>{
                if *shutdown.borrow(){return;}
                let target=match scope(&worker){Ok(s)=>s,Err(e)=>{CompletionWorker::report(Err(e));continue;}};
                let activate=current.as_ref()!=Some(&target);
                let owned=Arc::clone(&worker);
                let target_copy=target.clone();
                let cancelled=Arc::new(AtomicBool::new(false));
                let _cancel_page=CancelPage(Arc::clone(&cancelled));
                let result=tokio::task::spawn_blocking(move|| {
                    if cancelled.load(Ordering::Acquire) {return Ok(());}
                    if activate {store::activate(owned.queue.db_path(),&target_copy)?;}
                    reconcile(&owned,&target_copy,&cancelled)
                }).await;
                match result {
                    Ok(Ok(()))=>current=Some(target),
                    Ok(Err(error))=>{worker.server.mark_idle_observation_gap();CompletionWorker::report(Err(error));},
                    Err(error)=>{worker.server.mark_idle_observation_gap();eprintln!("observation_reconcile_join_error error={error}");},
                }
            }
        }
    }
}
