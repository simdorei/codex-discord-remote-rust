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
        let Some(turn_id) = job.turn_id else {
            return Ok(());
        };
        let result = self
            .server
            .execute(
                full_history_request(thread_id, self.history_read_timeout),
                Some(generation),
            )
            .await?;
        let states = parse_thread_turn_states(&result, thread_id)?;
        if let Some(completion) = states.get(&turn_id) {
            let evidence_generation = job.app_server_generation;
            self.finish(generation, evidence_generation, completion)
                .await?;
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
            return self
                .recover_goal_waiting(generation, &job.target_thread_id)
                .await;
        }
        let Some(turn_id) = job.turn_id.as_deref() else {
            self.attach_active(&job.target_thread_id).await?;
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
            self.finish(generation, job.app_server_generation, completion)
                .await?;
        }
        Ok(())
    }

    async fn recover_goal_waiting(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<(), CompletionWorkerError> {
        if self.attach_active(thread_id).await? {
            return Ok(());
        }
        if self.goal_status(generation, thread_id).await? != Some(ThreadGoalStatus::Active) {
            self.finish_waiting_goal(generation, thread_id).await?;
        }
        Ok(())
    }

    async fn attach_active(&self, thread_id: &str) -> Result<bool, CompletionWorkerError> {
        let Some(active) = self.server.active_turn_id(thread_id).await? else {
            return Ok(false);
        };
        Ok(self.queue.goal_turn_started(thread_id, &active).await?)
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
