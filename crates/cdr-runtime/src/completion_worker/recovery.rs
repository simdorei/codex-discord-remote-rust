use std::collections::BTreeSet;

use cdr_app_server::goal::ThreadGoalStatus;
use cdr_app_server::outcomes::{TurnStatus, parse_thread_turn_states};
use cdr_store::queue::{QueueJobState, StoredQueueJob, list};

use super::{CompletionWorker, CompletionWorkerError};
use crate::completion_worker::history_request::full_history_request;
use crate::queue_recovery_transport::stabilize_after_queue_recovery;

impl CompletionWorker {
    pub(super) async fn finish_waiting_goal(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<(), CompletionWorkerError> {
        let Some(job) = list(self.queue.db_path())?.into_iter().find(|job| {
            job.state == QueueJobState::Running
                && job.target_thread_id == thread_id
                && job.goal_waiting
        }) else {
            return Ok(());
        };
        self.finish_waiting_goal_owned(generation, &job).await
    }

    pub(super) async fn finish_waiting_goal_owned(
        &self,
        generation: u64,
        expected: &StoredQueueJob,
    ) -> Result<(), CompletionWorkerError> {
        let completion = self.waiting_goal_completion(generation, expected).await?;
        self.finish_with_owner(
            generation,
            expected.completion_evidence_generation(),
            &completion,
            Some(expected),
        )
        .await
    }

    /// A completed Goal does not identify an unobserved successor. History is
    /// reconciliation evidence, never permission to choose the newest turn.
    pub(super) async fn waiting_goal_completion(
        &self,
        generation: u64,
        expected: &StoredQueueJob,
    ) -> Result<cdr_app_server::outcomes::TurnCompletion, CompletionWorkerError> {
        let held = |reason: &str| {
            CompletionWorkerError::Held(format!(
                "Goal completion held for {}: {reason}; original progress owner retained",
                expected.target_thread_id,
            ))
        };
        if !expected.goal_waiting || expected.state != QueueJobState::Running {
            return Err(held("not the exact waiting owner"));
        }
        let turn = expected
            .turn_id
            .as_deref()
            .ok_or_else(|| held("missing prior turn"))?;
        let result = self
            .server
            .execute(
                full_history_request(&expected.target_thread_id, self.history_read_timeout),
                Some(generation),
            )
            .await?;
        let states = parse_thread_turn_states(&result, &expected.target_thread_id)?;
        if result["thread"]["turns"].as_array().map(Vec::len) != Some(states.len()) {
            return Err(held("duplicate history turn identity"));
        }
        let completion = states
            .get(turn)
            .ok_or_else(|| held("prior turn absent from history"))?;
        if completion.status == TurnStatus::InProgress {
            return Err(held("prior turn is not terminal"));
        }
        for (id, state) in &states {
            if id == turn || expected.baseline_turn_ids.contains(id) {
                continue;
            }
            // Completed predecessor turns have durable bot completion markers.
            // An unfamiliar successor, active OR already completed, must first
            // acquire exact ownership through a validated start observation.
            if state.status == TurnStatus::InProgress
                || !cdr_store::mirror::has_event(
                    self.queue.db_path(),
                    &cdr_store::mirror::turn_origin_marker(&expected.target_thread_id, id),
                    &expected.target_thread_id,
                )?
            {
                return Err(held("unattached turn requires an exact start observation"));
            }
        }
        let owners: Vec<_> = list(self.queue.db_path())?
            .into_iter()
            .filter(|job| {
                job.target_thread_id == expected.target_thread_id
                    && job.state == QueueJobState::Running
            })
            .collect();
        if generation != self.server.generation()
            || owners.as_slice() != [expected.clone()]
            || cdr_store::dead_generation::target_is_held(
                self.queue.db_path(),
                &expected.target_thread_id,
            )?
        {
            return Err(held(
                "resident or waiting ownership changed during history read",
            ));
        }
        Ok(completion.clone())
    }

    /// A thread-level terminal Goal status cannot identify this turn as the last.
    /// Reconcile even after attachment cleared `goal_waiting`. History only blocks
    /// Final / permits progress handoff; it never selects the successor's owner.
    pub(super) fn goal_history_has_unattached_turn(
        &self,
        result: &serde_json::Value,
        expected: &StoredQueueJob,
        completion: &cdr_app_server::outcomes::TurnCompletion,
    ) -> Result<bool, CompletionWorkerError> {
        let states = parse_thread_turn_states(result, &expected.target_thread_id)?;
        if result["thread"]["turns"].as_array().map(Vec::len) != Some(states.len()) {
            return Err(CompletionWorkerError::Held(
                "duplicate history turn identity".into(),
            ));
        }
        if states
            .get(&completion.turn_id)
            .is_some_and(|turn| turn.status != completion.status)
        {
            return Err(CompletionWorkerError::Held(
                "owned terminal status changed in history".into(),
            ));
        }
        for (id, state) in &states {
            if id == &completion.turn_id || expected.baseline_turn_ids.contains(id) {
                continue;
            }
            if state.status == TurnStatus::InProgress
                || !cdr_store::mirror::has_event(
                    self.queue.db_path(),
                    &cdr_store::mirror::turn_origin_marker(&expected.target_thread_id, id),
                    &expected.target_thread_id,
                )?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn require_completion_owner(
        &self,
        generation: u64,
        expected: &StoredQueueJob,
    ) -> Result<(), CompletionWorkerError> {
        let owners: Vec<_> = list(self.queue.db_path())?
            .into_iter()
            .filter(|job| {
                job.target_thread_id == expected.target_thread_id
                    && job.state == QueueJobState::Running
            })
            .collect();
        if generation != self.server.generation()
            || owners.as_slice() != [expected.clone()]
            || cdr_store::dead_generation::target_is_held(
                self.queue.db_path(),
                &expected.target_thread_id,
            )?
        {
            return Err(CompletionWorkerError::Held(
                "resident or completion ownership changed during history read".into(),
            ));
        }
        Ok(())
    }

    pub(super) async fn recover(&self) -> Result<(), CompletionWorkerError> {
        run_recovery_phases(
            || self.deliver_pending(),
            || self.recover_queue(),
            |error| eprintln!("completion_pending_recovery_error: {error}"),
        )
        .await
    }

    async fn recover_queue(&self) -> Result<(), CompletionWorkerError> {
        let (_admission, draining) = match self.queue.enter_background_recovery() {
            Ok(Some((permit, draining))) => (Some(permit), draining),
            Ok(None) => (None, false),
            Err(crate::queue_runner::QueueRunnerError::RestartDrain(
                crate::restart_readiness::drain::DrainGateError::Sealed,
            )) => {
                eprintln!("completion_queue_recovery_paused_for_restart_drain");
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        run_queue_recovery_strategy(
            draining,
            || self.mutate_queue_recovery(),
            |read_unavailable_targets| async move {
                self.reconcile_running(&read_unavailable_targets).await
            },
        )
        .await
    }

    async fn mutate_queue_recovery(&self) -> Result<BTreeSet<String>, CompletionWorkerError> {
        let queue_recovery = self.queue.recover().await?;
        if queue_recovery != crate::queue_runner::RecoveryReport::default() {
            eprintln!("rust_queue_recovery_retry: {queue_recovery:?}");
        }
        if stabilize_after_queue_recovery(self.server.as_ref()).await? {
            eprintln!(
                "rust_queue_recovery_retry_app_server_restarted generation={}",
                self.server.generation()
            );
        }
        Ok(queue_recovery.read_unavailable_targets)
    }

    async fn reconcile_running(
        &self,
        read_unavailable_targets: &BTreeSet<String>,
    ) -> Result<(), CompletionWorkerError> {
        let generation = self.server.generation();
        let jobs = list(self.queue.db_path())?
            .into_iter()
            .filter(|job| recoverable(job.state, &job.target_thread_id, read_unavailable_targets));
        attempt_all_recoveries(jobs, |job| self.recover_job(generation, job)).await
    }

    async fn recover_job(
        &self,
        generation: u64,
        job: StoredQueueJob,
    ) -> Result<(), CompletionWorkerError> {
        if job.goal_waiting {
            return self.recover_goal_waiting(generation, &job).await;
        }
        let Some(turn_id) = job.turn_id.as_deref() else {
            self.attach_active(&job).await?;
            return Ok(());
        };
        let result = self
            .server
            .execute(
                full_history_request(&job.target_thread_id, self.history_read_timeout),
                Some(generation),
            )
            .await?;
        let states = parse_thread_turn_states(&result, &job.target_thread_id)?;
        if let Some(completion) = states.get(turn_id)
            && completion.status != TurnStatus::InProgress
        {
            self.finish_with_owner(
                generation,
                job.completion_evidence_generation(),
                completion,
                Some(&job),
            )
            .await?;
        }
        Ok(())
    }

    async fn recover_goal_waiting(
        &self,
        generation: u64,
        job: &StoredQueueJob,
    ) -> Result<(), CompletionWorkerError> {
        let thread_id = &job.target_thread_id;
        if self.attach_active(job).await? {
            return Ok(());
        }
        if self.goal_status(generation, thread_id).await? != Some(ThreadGoalStatus::Active) {
            self.finish_waiting_goal_owned(generation, job).await?;
        }
        Ok(())
    }

    async fn attach_active(&self, job: &StoredQueueJob) -> Result<bool, CompletionWorkerError> {
        let thread_id = &job.target_thread_id;
        let observation_generation = self.server.generation();
        let Some(active) = self.server.active_turn_id(thread_id).await? else {
            return Ok(false);
        };
        Ok(self
            .queue
            .goal_turn_started_observed(thread_id, &active, observation_generation, Some(job))
            .await?)
    }
}

async fn run_queue_recovery_strategy<E, M, MFut, R, RFut>(
    draining: bool,
    mutate: M,
    reconcile: R,
) -> Result<(), E>
where
    M: FnOnce() -> MFut,
    MFut: std::future::Future<Output = Result<BTreeSet<String>, E>>,
    R: FnOnce(BTreeSet<String>) -> RFut,
    RFut: std::future::Future<Output = Result<(), E>>,
{
    let unavailable = if draining {
        BTreeSet::new()
    } else {
        mutate().await?
    };
    reconcile(unavailable).await
}

fn recoverable(
    state: QueueJobState,
    target: &str,
    read_unavailable_targets: &BTreeSet<String>,
) -> bool {
    state == QueueJobState::Running && !read_unavailable_targets.contains(target)
}

async fn attempt_all_recoveries<I, E, F, Fut>(
    items: impl IntoIterator<Item = I>,
    mut attempt: F,
) -> Result<(), E>
where
    F: FnMut(I) -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
{
    let mut first_error = None;
    for item in items {
        if let Err(error) = attempt(item).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn run_recovery_phases<E, PF, PFut, QF, QFut, S>(
    pending: PF,
    queue: QF,
    mut surface_pending: S,
) -> Result<(), E>
where
    PF: FnOnce() -> PFut,
    PFut: std::future::Future<Output = Result<(), E>>,
    QF: FnOnce() -> QFut,
    QFut: std::future::Future<Output = Result<(), E>>,
    S: FnMut(&E),
{
    let pending_error = pending().await.err();
    let queue_result = queue().await;
    match (pending_error, queue_result) {
        (Some(pending), Err(queue)) => {
            surface_pending(&pending);
            Err(queue)
        }
        (Some(pending), Ok(())) => Err(pending),
        (None, queue) => queue,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use cdr_app_server::{AppServerError, ResidentLifecycleSnapshot};

    use super::*;

    struct QuarantinedCompletionRecoveryServer {
        restart_calls: AtomicUsize,
    }

    impl crate::queue_recovery_transport::QueueRecoveryServer for QuarantinedCompletionRecoveryServer {
        async fn recovery_snapshot(&self) -> ResidentLifecycleSnapshot {
            ResidentLifecycleSnapshot {
                generation: 3,
                healthy: false,
                quarantined: true,
                restart_pending: true,
                process_id: Some(9),
            }
        }

        async fn force_recovery_restart(&self) -> Result<bool, AppServerError> {
            self.restart_calls.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        }
    }

    #[tokio::test]
    async fn quarantined_completion_recovery_refreshes_transport_before_next_tick() {
        let server = QuarantinedCompletionRecoveryServer {
            restart_calls: AtomicUsize::new(0),
        };

        assert!(
            stabilize_after_queue_recovery(&server)
                .await
                .expect("restart quarantined completion recovery transport")
        );
        assert_eq!(server.restart_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn completion_retry_only_skips_a_target_that_queue_recovery_could_not_read() {
        let report = crate::queue_runner::RecoveryReport {
            read_unavailable_targets: BTreeSet::from(["thread-a".to_owned()]),
            mutation_unavailable_targets: BTreeSet::from(["thread-b".to_owned()]),
            unavailable_targets: BTreeSet::from(["thread-a".to_owned(), "thread-b".to_owned()]),
            ..crate::queue_runner::RecoveryReport::default()
        };

        assert!(!recoverable(
            QueueJobState::Running,
            "thread-a",
            &report.read_unavailable_targets
        ));
        assert!(recoverable(
            QueueJobState::Running,
            "thread-b",
            &report.read_unavailable_targets
        ));
        assert!(!recoverable(
            QueueJobState::Pending,
            "thread-b",
            &report.read_unavailable_targets
        ));
    }

    #[tokio::test]
    async fn queue_recovery_runs_after_pending_delivery_failure() {
        let queue_ran = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&queue_ran);

        let result = run_recovery_phases(
            || async { Err::<(), _>("persistent Discord failure") },
            || async move {
                observed.store(true, Ordering::SeqCst);
                Ok(())
            },
            |_| {},
        )
        .await;

        assert_eq!(result, Err("persistent Discord failure"));
        assert!(queue_ran.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn sealed_drain_skips_mutation_but_reconciles_missed_completion() {
        let mutated = Arc::new(AtomicBool::new(false));
        let reconciled = Arc::new(AtomicBool::new(false));
        let mutation_observer = Arc::clone(&mutated);
        let reconciliation_observer = Arc::clone(&reconciled);

        run_queue_recovery_strategy(
            true,
            || async move {
                mutation_observer.store(true, Ordering::SeqCst);
                Ok::<_, &'static str>(BTreeSet::from(["unavailable".into()]))
            },
            |unavailable| async move {
                assert!(unavailable.is_empty());
                reconciliation_observer.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();

        assert!(!mutated.load(Ordering::SeqCst));
        assert!(reconciled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn running_job_batch_attempts_every_job_and_returns_the_first_exact_error() {
        for failed_stage in ["goal", "read", "parse", "finish"] {
            let attempts = Arc::new(Mutex::new(Vec::new()));
            let progress = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&attempts);
            let advanced = Arc::clone(&progress);

            let error = attempt_all_recoveries(
                [failed_stage, "later-success", "later-failure"],
                move |stage| {
                    recorded.lock().unwrap().push(stage);
                    let advanced = Arc::clone(&advanced);
                    async move {
                        match stage {
                            "later-success" => {
                                advanced.lock().unwrap().push(stage);
                                Ok(())
                            }
                            "later-failure" => Err("later exact error"),
                            _ => Err(stage),
                        }
                    }
                },
            )
            .await
            .unwrap_err();

            assert_eq!(error, failed_stage);
            assert_eq!(
                attempts.lock().unwrap().as_slice(),
                &[failed_stage, "later-success", "later-failure"]
            );
            assert_eq!(progress.lock().unwrap().as_slice(), &["later-success"]);
        }
    }
}
