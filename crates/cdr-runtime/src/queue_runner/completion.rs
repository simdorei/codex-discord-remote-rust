use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use cdr_store::delivery::{StoredDelivery, stage_queue_completion_with_release};
use cdr_store::queue::{
    QueueJobState, attach_goal_turn, complete, list_filtered, mark_goal_waiting,
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
        self.stage_turn_completion_on_generation(
            target_thread_id,
            turn_id,
            content,
            generation_i64(self.backend.generation())?,
        )
        .await
    }

    pub async fn stage_turn_completion_on_generation(
        &self,
        target_thread_id: &str,
        turn_id: &str,
        content: &str,
        evidence_generation: i64,
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
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
        let delivery = stage_queue_completion_with_release(
            &self.db_path,
            &job.job_id,
            content,
            now,
            self.backend
                .resident_instance_id()
                .filter(|_| evidence_generation == generation)
                .map(|owner| (owner, generation)),
        )?;
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
        let generation = generation_i64(self.backend.generation())?;
        let jobs = list_filtered(&self.db_path, Some(target), Some(generation))?;
        let job = jobs
            .iter()
            .find(|job| job.state == QueueJobState::Running && job.turn_id.as_deref() == Some(turn))
            .ok_or_else(|| cdr_store::StoreError::QueueJobNotFound(turn.into()))?;
        Ok(cdr_store::goal_progress::stage(
            &self.db_path,
            &job.job_id,
            turn,
            generation,
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
        let generation = generation_i64(self.backend.generation())?;
        let jobs = list_filtered(&self.db_path, Some(target_thread_id), Some(generation))?;
        let Some(job) = jobs.iter().find(|job| {
            job.state == QueueJobState::Running && job.turn_id.as_deref() == Some(turn_id)
        }) else {
            return Ok(false);
        };
        Ok(mark_goal_waiting(
            &self.db_path,
            &job.job_id,
            turn_id,
            generation,
        )?)
    }

    pub async fn goal_turn_started(
        &self,
        target_thread_id: &str,
        turn_id: &str,
    ) -> Result<bool, QueueRunnerError> {
        let lock = self.target_lock(target_thread_id)?;
        let _guard = lock.lock().await;
        if cdr_store::dead_generation::target_is_held(&self.db_path, target_thread_id)? {
            return Ok(false);
        }
        Ok(attach_goal_turn(
            &self.db_path,
            target_thread_id,
            turn_id,
            generation_i64(self.backend.generation())?,
        )?)
    }
}
