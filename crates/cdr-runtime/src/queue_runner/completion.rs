use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use cdr_store::delivery::{StoredDelivery, stage_owned_queue_completion_with_release};
use cdr_store::queue::{
    QueueJobState, attach_goal_turn_observed_if_owned, complete, list_filtered, mark_goal_waiting,
};

use super::{QueueCoordinator, QueueRunnerError, TurnBackend, generation_i64};

struct CompletionEvidence<'a> {
    expected_job: Option<&'a cdr_store::queue::StoredQueueJob>,
    usage_limit: bool,
    observed_generation: Option<i64>,
}

impl<B: TurnBackend> QueueCoordinator<B> {
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub async fn turn_completed(
        &self,
        target_thread_id: &str,
        turn_id: &str,
    ) -> Result<bool, QueueRunnerError> {
        let lock = self.target_lock(target_thread_id)?;
        let _guard = lock.lock().await;
        if cdr_store::dead_generation::target_is_held(&self.db_path, target_thread_id)? {
            return Ok(false);
        }
        let generation = generation_i64(self.backend.generation())?;
        let jobs = list_filtered(&self.db_path, Some(target_thread_id), None)?;
        let matched = jobs.iter().find(|job| {
            job.state == QueueJobState::Running && job.turn_id.as_deref() == Some(turn_id)
        });
        let removed = if let Some(job) = matched {
            complete(&self.db_path, &job.job_id)?
        } else {
            false
        };
        if removed {
            let _ = self.start_next_locked(target_thread_id, generation).await?;
        }
        Ok(removed)
    }

    /// A compatibility caller without observed terminal evidence has no release authority.
    pub async fn stage_turn_completion(
        &self,
        target: &str,
        turn: &str,
        content: &str,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        self.stage_turn_completion_with_usage_limit(target, turn, content, false)
            .await
    }

    pub async fn stage_turn_completion_with_usage_limit(
        &self,
        target: &str,
        turn: &str,
        content: &str,
        usage_limit: bool,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        self.stage_turn_completion_inner(
            target,
            turn,
            content,
            CompletionEvidence {
                expected_job: None,
                usage_limit,
                observed_generation: None,
            },
        )
        .await
    }

    pub async fn stage_turn_completion_on_generation(
        &self,
        target: &str,
        turn: &str,
        content: &str,
        generation: i64,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        self.stage_turn_completion_inner(
            target,
            turn,
            content,
            CompletionEvidence {
                expected_job: None,
                usage_limit: false,
                observed_generation: Some(generation),
            },
        )
        .await
    }

    pub async fn stage_owned_turn_completion(
        &self,
        expected: &cdr_store::queue::StoredQueueJob,
        content: &str,
        usage_limit: bool,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        self.stage_owned_turn_completion_observed(expected, content, usage_limit, None)
            .await
    }

    pub(crate) async fn stage_owned_turn_completion_observed(
        &self,
        expected: &cdr_store::queue::StoredQueueJob,
        content: &str,
        usage_limit: bool,
        observed_generation: Option<i64>,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        let turn = expected.turn_id.as_deref().ok_or_else(|| {
            cdr_store::StoreError::InvalidQueueState("completion owner has no turn".into())
        })?;
        self.stage_turn_completion_inner(
            &expected.target_thread_id,
            turn,
            content,
            CompletionEvidence {
                expected_job: Some(expected),
                usage_limit,
                observed_generation,
            },
        )
        .await
    }

    async fn stage_turn_completion_inner(
        &self,
        target: &str,
        turn: &str,
        content: &str,
        evidence: CompletionEvidence<'_>,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        let lock = self.target_lock(target)?;
        let _guard = lock.lock().await;
        if cdr_store::dead_generation::target_is_held(&self.db_path, target)? {
            return Ok(None);
        }
        // Snapshot current control authority separately from the historical job.
        let before_generation = generation_i64(self.backend.generation())?;
        let before_resident = self.backend.resident_instance_id().map(str::to_owned);
        let jobs = list_filtered(&self.db_path, Some(target), None)?;
        let mut owners = jobs.iter().filter(|job| {
            job.state == QueueJobState::Running && job.turn_id.as_deref() == Some(turn)
        });
        let Some(job) = owners.next() else {
            return Ok(None);
        };
        if owners.next().is_some()
            || evidence
                .expected_job
                .is_some_and(|expected| expected != job)
        {
            return Err(cdr_store::StoreError::InvalidQueueState(
                "completion ownership changed during observation".into(),
            )
            .into());
        }
        if evidence.usage_limit {
            cdr_store::reserve_policy::stage_usage_failure(
                &self.db_path,
                target,
                "terminal typed usage-limit failure requires automatic handling",
            )?;
            // The failure must remain durable even if current settings preparation fails.
            let _ = self.backend.note_usage_limit(target).await;
        }
        let generation = generation_i64(self.backend.generation())?;
        let release_owner = before_resident
            .as_deref()
            .filter(|owner| {
                evidence.observed_generation == Some(generation)
                    && generation == before_generation
                    && self.backend.resident_instance_id() == Some(*owner)
                    && job.completion_evidence_generation() == generation
            })
            .map(|owner| (owner, generation));
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
        // Revalidate the ENTIRE captured row in the same transaction as its
        // consumption. An await above cannot license a newer owner by job_id.
        let delivery = stage_owned_queue_completion_with_release(
            &self.db_path,
            job,
            content,
            now,
            release_owner,
        )?;
        self.notify_delivery_ready();
        let _ = self.start_next_locked(target, generation).await?;
        Ok(Some(delivery))
    }

    pub async fn stage_goal_progress(
        &self,
        target: &str,
        turn: &str,
        content: &str,
    ) -> Result<Option<cdr_store::goal_progress::PendingProgress>, QueueRunnerError> {
        let lock = self.target_lock(target)?;
        let _guard = lock.lock().await;
        // Running generations are execution evidence and survive resident changes.
        // Keep the store's exact ownership check, using the persisted generation.
        let jobs = list_filtered(&self.db_path, Some(target), None)?;
        let job = jobs
            .iter()
            .find(|job| job.state == QueueJobState::Running && job.turn_id.as_deref() == Some(turn))
            .ok_or_else(|| cdr_store::StoreError::QueueJobNotFound(turn.into()))?;
        Ok(cdr_store::goal_progress::stage(
            &self.db_path,
            &job.job_id,
            turn,
            job.app_server_generation,
            content,
        )?)
    }

    pub(crate) async fn stage_owned_goal_progress(
        &self,
        expected: &cdr_store::queue::StoredQueueJob,
        content: &str,
    ) -> Result<Option<cdr_store::goal_progress::PendingProgress>, QueueRunnerError> {
        let lock = self.target_lock(&expected.target_thread_id)?;
        let _guard = lock.lock().await;
        // The store revalidates the entire captured owner and uniqueness in the
        // same transaction as progress, journal consumption and waiting handoff.
        Ok(cdr_store::goal_progress::stage_owned(
            &self.db_path,
            expected,
            content,
        )?)
    }

    pub async fn goal_continues(
        &self,
        target_thread_id: &str,
        turn_id: &str,
    ) -> Result<bool, QueueRunnerError> {
        let lock = self.target_lock(target_thread_id)?;
        let _guard = lock.lock().await;
        if cdr_store::dead_generation::target_is_held(&self.db_path, target_thread_id)? {
            return Ok(false);
        }
        let jobs = list_filtered(&self.db_path, Some(target_thread_id), None)?;
        let Some(job) = jobs.iter().find(|job| {
            job.state == QueueJobState::Running && job.turn_id.as_deref() == Some(turn_id)
        }) else {
            return Ok(false);
        };
        Ok(mark_goal_waiting(
            &self.db_path,
            &job.job_id,
            turn_id,
            job.app_server_generation,
        )?)
    }

    pub async fn goal_turn_started(
        &self,
        target_thread_id: &str,
        turn_id: &str,
    ) -> Result<bool, QueueRunnerError> {
        self.goal_turn_started_observed(target_thread_id, turn_id, self.backend.generation(), None)
            .await
    }

    pub(crate) async fn goal_turn_started_observed(
        &self,
        target_thread_id: &str,
        turn_id: &str,
        observation_generation: u64,
        expected_owner: Option<&cdr_store::queue::StoredQueueJob>,
    ) -> Result<bool, QueueRunnerError> {
        let lock = self.target_lock(target_thread_id)?;
        let _guard = lock.lock().await;
        // Validate the live observation, not the historical job generation.
        // Recheck after waiting for the target lock so an old event cannot bind
        // merely because an inherited job is now visible across generations.
        if observation_generation != self.backend.generation()
            || cdr_store::dead_generation::target_is_held(&self.db_path, target_thread_id)?
        {
            return Ok(false);
        }
        let jobs = list_filtered(&self.db_path, Some(target_thread_id), None)?;
        let mut waiting = jobs
            .iter()
            .filter(|job| job.state == QueueJobState::Running && job.goal_waiting);
        let Some(job) = waiting.next() else {
            return Ok(false);
        };
        if waiting.next().is_some() {
            return Err(cdr_store::StoreError::InvalidQueueState(format!(
                "multiple goal-waiting jobs for {target_thread_id}"
            ))
            .into());
        }
        if expected_owner.is_some_and(|expected| expected != job) {
            return Ok(false);
        }
        Ok(attach_goal_turn_observed_if_owned(
            &self.db_path,
            job,
            turn_id,
            generation_i64(observation_generation)?,
        )?)
    }
}
