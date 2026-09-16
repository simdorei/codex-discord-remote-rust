use std::time::{SystemTime, UNIX_EPOCH};

use cdr_store::queue::{
    QueueJobState, STARTING_CANDIDATE_HOLD_PREFIX, StoredQueueJob, UNRESOLVED_FORK_ERROR_PREFIX,
    list_filtered, mark_running_if_claimed, record_preflight_failure,
    record_start_failure_if_claimed, try_begin_attempt,
};

use super::{
    BackendFailure, QueueCoordinator, QueueRunnerError, Submission, TurnBackend, TurnRecord,
};

#[must_use]
pub const fn retry_delay_seconds(failure_count: i64) -> u32 {
    match failure_count {
        i64::MIN..=0 => 0,
        1 => 30,
        2 => 60,
        3 => 120,
        4 => 240,
        5 => 480,
        6..=i64::MAX => 900,
    }
}

#[must_use]
pub fn pending_retry_due_at(failure_count: i64, last_error: &str, updated_at: f64) -> Option<f64> {
    if failure_count <= 0 || last_error.is_empty() || !updated_at.is_finite() {
        return None;
    }
    let due_at = updated_at + f64::from(retry_delay_seconds(failure_count));
    due_at.is_finite().then_some(due_at)
}

#[must_use]
pub fn pending_retry_is_due(
    failure_count: i64,
    last_error: &str,
    updated_at: f64,
    now: f64,
) -> bool {
    pending_retry_due_at(failure_count, last_error, updated_at)
        .is_none_or(|due_at| now.is_finite() && now >= due_at)
}

pub(super) fn unix_now() -> Result<f64, QueueRunnerError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

pub(super) fn eligible_pending_head(jobs: &[StoredQueueJob]) -> Option<&StoredQueueJob> {
    if jobs
        .iter()
        .any(|job| matches!(job.state, QueueJobState::Starting | QueueJobState::Running))
    {
        return None;
    }
    jobs.iter().find(|job| {
        job.state == QueueJobState::Pending
            && !job
                .last_error
                .starts_with(cdr_store::reserve_policy::HOLD_PREFIX)
    })
}

pub(super) fn pending_job_is_due(job: &StoredQueueJob, now: f64) -> bool {
    pending_retry_is_due(job.attempt_count, &job.last_error, job.updated_at, now)
}

pub(super) fn replay_existing(job: StoredQueueJob) -> Submission {
    let quarantined = job.state == QueueJobState::Quarantined;
    let fork_fenced = job.last_error.starts_with(UNRESOLVED_FORK_ERROR_PREFIX);
    let starting_candidates_held = job.last_error.starts_with(STARTING_CANDIDATE_HOLD_PREFIX);
    let auto_reserve_held = job
        .last_error
        .starts_with(cdr_store::reserve_policy::HOLD_PREFIX);
    let warning = if quarantined {
        Some(BackendFailure::quarantined(job.last_error))
    } else if fork_fenced {
        Some(BackendFailure::fork_fenced(job.last_error))
    } else if starting_candidates_held {
        Some(BackendFailure::starting_candidates_held(job.last_error))
    } else if auto_reserve_held {
        Some(BackendFailure::auto_reserve_held(job.last_error))
    } else {
        (!job.last_error.is_empty()).then(|| {
            BackendFailure::persisted(job.last_error, job.state == QueueJobState::Starting)
        })
    };
    Submission {
        job_id: job.job_id,
        queued: !quarantined && !starting_candidates_held && job.turn_id.is_none(),
        turn_id: if quarantined || starting_candidates_held {
            None
        } else {
            job.turn_id
        },
        warning,
    }
}

impl<B: TurnBackend> QueueCoordinator<B> {
    pub(crate) fn enter_background_recovery(
        &self,
    ) -> Result<Option<(crate::restart_readiness::drain::AdmissionPermit, bool)>, QueueRunnerError>
    {
        self.admission
            .as_ref()
            .map(|gate| gate.try_enter_control_observed().map_err(Into::into))
            .transpose()
    }

    pub(super) async fn start_next_locked(
        &self,
        target_thread_id: &str,
        generation: i64,
    ) -> Result<Option<StoredQueueJob>, QueueRunnerError> {
        self.start_next_with_baseline_locked(target_thread_id, generation, None)
            .await
    }

    pub(super) async fn start_next_recovered_locked(
        &self,
        target_thread_id: &str,
        generation: i64,
        turns: &[TurnRecord],
    ) -> Result<Option<StoredQueueJob>, QueueRunnerError> {
        self.start_next_with_baseline_locked(target_thread_id, generation, Some(turns))
            .await
    }

    async fn start_next_with_baseline_locked(
        &self,
        target_thread_id: &str,
        generation: i64,
        recovered_turns: Option<&[TurnRecord]>,
    ) -> Result<Option<StoredQueueJob>, QueueRunnerError> {
        let _admission = match &self.admission {
            Some(gate) => match gate.try_enter() {
                Ok(permit) => Some(permit),
                Err(crate::restart_readiness::drain::DrainGateError::Sealed) => return Ok(None),
                Err(error) => return Err(error.into()),
            },
            None => None,
        };
        if cdr_store::dead_generation::target_is_held(&self.db_path, target_thread_id)? {
            return Ok(None);
        }
        // A cold/old-generation Starting or Running job still owns this target.
        // Filtering it out before eligibility would let a current-generation
        // kick or completion start another request past the unresolved attempt.
        let jobs = list_filtered(&self.db_path, Some(target_thread_id), None)?;
        let Some(job) = eligible_pending_head(&jobs).cloned() else {
            return Ok(None);
        };
        // Only authoritative recovery may adopt an old Pending generation.
        // Do not bypass that head in order to start a newer queued request.
        if job.app_server_generation != generation {
            return Ok(None);
        }
        if !pending_job_is_due(&job, unix_now()?) {
            return Ok(None);
        }
        if self
            .backend
            .active_turn_id(target_thread_id)
            .await
            .map_err(QueueRunnerError::Backend)?
            .is_some()
        {
            return Ok(None);
        }
        self.backend
            .prepare_turn(target_thread_id)
            .await
            .map_err(QueueRunnerError::Backend)?;
        let baseline = self
            .preflight_baseline(target_thread_id, generation, &job, recovered_turns)
            .await?;
        let Some(claimed) = try_begin_attempt(&self.db_path, &job.job_id, &baseline, generation)?
        else {
            return Ok(None);
        };
        let turn_id = match self.backend.start_turn(target_thread_id, &job.prompt).await {
            Ok(turn_id) => turn_id,
            Err(error) => {
                let failure_message =
                    if error.kind == crate::queue_runner::BackendFailureKind::UsageLimit {
                        format!(
                            "{}{}",
                            cdr_store::reserve_policy::HOLD_PREFIX,
                            error.message
                        )
                    } else {
                        error.message.clone()
                    };
                let recorded = record_start_failure_if_claimed(
                    &self.db_path,
                    &claimed,
                    &failure_message,
                    error.ambiguous,
                )?;
                if recorded.is_none() {
                    return Err(QueueRunnerError::AttemptClaimLost {
                        job_id: job.job_id,
                        observed_turn_id: None,
                    });
                }
                if error.kind == crate::queue_runner::BackendFailureKind::UsageLimit {
                    // The notice and hold were committed atomically even when no
                    // foreground caller exists for this previously queued job.
                    self.notify_delivery_ready();
                    let _ = self.backend.note_usage_limit(target_thread_id).await;
                }
                return Err(error.into());
            }
        };
        let running = mark_running_if_claimed(&self.db_path, &claimed, &turn_id)?;
        match running {
            Some(job) => Ok(Some(job)),
            None => Err(QueueRunnerError::AttemptClaimLost {
                job_id: job.job_id,
                observed_turn_id: Some(turn_id),
            }),
        }
    }

    async fn preflight_baseline(
        &self,
        target_thread_id: &str,
        generation: i64,
        job: &StoredQueueJob,
        recovered_turns: Option<&[TurnRecord]>,
    ) -> Result<Vec<String>, QueueRunnerError> {
        if let Some(turns) = recovered_turns {
            return Ok(turns.iter().map(|turn| turn.turn_id.clone()).collect());
        }
        if let Err(error) = self.backend.resume_thread(target_thread_id).await {
            self.record_preflight(job, generation, &error)?;
            return Err(error.into());
        }
        match self.backend.read_turns(target_thread_id).await {
            Ok(turns) => Ok(turns.into_iter().map(|turn| turn.turn_id).collect()),
            Err(error) => {
                self.record_preflight(job, generation, &error)?;
                Err(error.into())
            }
        }
    }

    fn record_preflight(
        &self,
        job: &StoredQueueJob,
        generation: i64,
        error: &BackendFailure,
    ) -> Result<(), QueueRunnerError> {
        let _ = record_preflight_failure(&self.db_path, &job.job_id, generation, &error.message)?;
        Ok(())
    }
}
