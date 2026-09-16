use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use cdr_store::delivery::{StoredDelivery, stage_queue_completion};
use cdr_store::queue::{
    QueueJobState, attach_goal_turn_observed_if_owned, complete, list_filtered, mark_goal_waiting,
};

use super::{QueueCoordinator, QueueRunnerError, TurnBackend, generation_i64};

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

    pub async fn stage_turn_completion(
        &self,
        target_thread_id: &str,
        turn_id: &str,
        content: &str,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        self.stage_turn_completion_with_usage_limit(target_thread_id, turn_id, content, false)
            .await
    }

    pub async fn stage_turn_completion_with_usage_limit(
        &self,
        target_thread_id: &str,
        turn_id: &str,
        content: &str,
        usage_limit: bool,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        self.stage_turn_completion_inner(target_thread_id, turn_id, content, usage_limit, None)
            .await
    }

    pub(crate) async fn stage_owned_turn_completion(
        &self,
        expected: &cdr_store::queue::StoredQueueJob,
        content: &str,
        usage_limit: bool,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        let turn = expected.turn_id.as_deref().ok_or_else(|| {
            cdr_store::StoreError::InvalidQueueState("completion owner has no turn".into())
        })?;
        self.stage_turn_completion_inner(
            &expected.target_thread_id,
            turn,
            content,
            usage_limit,
            Some(expected),
        )
        .await
    }

    async fn stage_turn_completion_inner(
        &self,
        target_thread_id: &str,
        turn_id: &str,
        content: &str,
        usage_limit: bool,
        expected_owner: Option<&cdr_store::queue::StoredQueueJob>,
    ) -> Result<Option<StoredDelivery>, QueueRunnerError> {
        let lock = self.target_lock(target_thread_id)?;
        let _guard = lock.lock().await;
        if cdr_store::dead_generation::target_is_held(&self.db_path, target_thread_id)? {
            return Ok(None);
        }
        let generation = generation_i64(self.backend.generation())?;
        let jobs = list_filtered(&self.db_path, Some(target_thread_id), None)?;
        let Some(job) = jobs.iter().find(|job| {
            job.state == QueueJobState::Running && job.turn_id.as_deref() == Some(turn_id)
        }) else {
            return Ok(None);
        };
        if expected_owner.is_some_and(|expected| expected != job) {
            return Err(cdr_store::StoreError::InvalidQueueState(
                "completion ownership changed during observation".into(),
            )
            .into());
        }
        if usage_limit {
            cdr_store::reserve_policy::stage_usage_failure(
                &self.db_path,
                target_thread_id,
                "terminal typed usage-limit failure requires automatic handling",
            )?;
            // A historical completion requests a fresh policy check; its execution
            // generation (including legacy NULL) is evidence, not settings authority.
            // The controller pins the CURRENT resident through its own validation.
            // This never resubmits the completed job's input.
            let _ = self.backend.note_usage_limit(target_thread_id).await;
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
        let delivery = stage_queue_completion(&self.db_path, &job.job_id, content, now)?;
        self.notify_delivery_ready();
        let _ = self.start_next_locked(target_thread_id, generation).await?;
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
