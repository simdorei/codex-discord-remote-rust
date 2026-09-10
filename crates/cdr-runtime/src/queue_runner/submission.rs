use cdr_store::prompt_intake::{PromptIntakeClaim, promote_prompt_intake_to_queue};
use cdr_store::queue::{
    ExpectedMirrorMapping, NewQueueJob, QueueJobState, enqueue, enqueue_if_mirror_matches,
    list_filtered,
};
use uuid::Uuid;

use super::{QueueCoordinator, QueueRunnerError, Submission, TurnBackend, generation_i64, id_i64};

pub(super) struct SubmissionRequest<'a> {
    pub(super) job_id: &'a str,
    pub(super) target_thread_id: &'a str,
    pub(super) channel_id: u64,
    pub(super) owner_user_id: u64,
    pub(super) discord_message_id: Option<u64>,
    pub(super) prompt: &'a str,
    pub(super) require_current_mirror: bool,
    pub(super) intake_claim: Option<&'a PromptIntakeClaim>,
}

impl<B: TurnBackend> QueueCoordinator<B> {
    pub async fn submit(
        &self,
        target_thread_id: &str,
        channel_id: u64,
        owner_user_id: u64,
        discord_message_id: Option<u64>,
        prompt: &str,
    ) -> Result<Submission, QueueRunnerError> {
        let job_id = Uuid::new_v4().to_string();
        self.submit_identified(
            &job_id,
            target_thread_id,
            channel_id,
            owner_user_id,
            discord_message_id,
            prompt,
        )
        .await
    }

    pub async fn submit_identified(
        &self,
        job_id: &str,
        target_thread_id: &str,
        channel_id: u64,
        owner_user_id: u64,
        discord_message_id: Option<u64>,
        prompt: &str,
    ) -> Result<Submission, QueueRunnerError> {
        self.submit_with_mapping(SubmissionRequest {
            job_id,
            target_thread_id,
            channel_id,
            owner_user_id,
            discord_message_id,
            prompt,
            require_current_mirror: false,
            intake_claim: None,
        })
        .await
    }

    pub async fn submit_mirror(
        &self,
        target_thread_id: &str,
        channel_id: u64,
        owner_user_id: u64,
        discord_message_id: Option<u64>,
        prompt: &str,
    ) -> Result<Submission, QueueRunnerError> {
        let job_id = Uuid::new_v4().to_string();
        self.submit_mirror_identified(
            &job_id,
            target_thread_id,
            channel_id,
            owner_user_id,
            discord_message_id,
            prompt,
        )
        .await
    }

    pub async fn submit_mirror_identified(
        &self,
        job_id: &str,
        target_thread_id: &str,
        channel_id: u64,
        owner_user_id: u64,
        discord_message_id: Option<u64>,
        prompt: &str,
    ) -> Result<Submission, QueueRunnerError> {
        self.submit_with_mapping(SubmissionRequest {
            job_id,
            target_thread_id,
            channel_id,
            owner_user_id,
            discord_message_id,
            prompt,
            require_current_mirror: true,
            intake_claim: None,
        })
        .await
    }

    pub(super) async fn submit_with_mapping(
        &self,
        request: SubmissionRequest<'_>,
    ) -> Result<Submission, QueueRunnerError> {
        let lock = self.target_lock(request.target_thread_id)?;
        let _guard = lock.lock().await;
        self.ensure_target_not_held(request.target_thread_id)?;
        let generation = generation_i64(self.backend.generation())?;
        let existing = list_filtered(&self.db_path, Some(request.target_thread_id), None)?;
        let queued = existing
            .iter()
            .any(|job| job.state != QueueJobState::Quarantined);
        let needs_generation_recovery = existing.iter().any(|job| {
            job.state != QueueJobState::Quarantined && job.app_server_generation != generation
        });
        let created_at = super::retry::unix_now()?;
        let channel_id = id_i64(request.channel_id)?;
        let new_job = NewQueueJob {
            job_id: request.job_id,
            target_thread_id: request.target_thread_id,
            channel_id,
            owner_user_id: Some(id_i64(request.owner_user_id)?),
            discord_message_id: request.discord_message_id.map(id_i64).transpose()?,
            app_server_generation: generation,
            prompt: request.prompt,
            queued,
            ack_sent: true,
            created_at,
        };
        let enqueued = if let Some(claim) = request.intake_claim {
            promote_prompt_intake_to_queue(&self.db_path, claim, new_job, created_at)?
        } else if request.require_current_mirror {
            enqueue_if_mirror_matches(
                &self.db_path,
                new_job,
                ExpectedMirrorMapping {
                    discord_channel_id: channel_id,
                    target_thread_id: request.target_thread_id,
                },
            )?
        } else {
            enqueue(&self.db_path, new_job)?
        };
        if !enqueued.created {
            return Ok(super::retry::replay_existing(enqueued.job));
        }
        if needs_generation_recovery {
            return Ok(Submission {
                job_id: enqueued.job.job_id,
                queued: true,
                turn_id: None,
                warning: None,
            });
        }
        let started = match self
            .start_next_locked(request.target_thread_id, generation)
            .await
        {
            Ok(started) => started,
            Err(QueueRunnerError::Backend(failure)) => {
                let current = list_filtered(
                    &self.db_path,
                    Some(request.target_thread_id),
                    Some(generation),
                )?
                .into_iter()
                .find(|job| job.job_id == enqueued.job.job_id);
                if let Some(job) = current
                    && !job.last_error.is_empty()
                {
                    return Ok(super::retry::replay_existing(job));
                }
                return Err(failure.into());
            }
            Err(error) => return Err(error),
        };
        let turn_id = started
            .filter(|job| job.job_id == enqueued.job.job_id)
            .and_then(|job| job.turn_id);
        Ok(Submission {
            job_id: enqueued.job.job_id,
            queued: turn_id.is_none(),
            turn_id,
            warning: None,
        })
    }

    pub fn replay_submission_for_message(
        &self,
        discord_message_id: u64,
    ) -> Result<Option<Submission>, QueueRunnerError> {
        let message_id = id_i64(discord_message_id)?;
        Ok(list_filtered(&self.db_path, None, None)?
            .into_iter()
            .find(|job| job.discord_message_id == Some(message_id))
            .map(super::retry::replay_existing))
    }

    pub fn replay_submission_for_job(
        &self,
        job_id: &str,
    ) -> Result<Option<Submission>, QueueRunnerError> {
        Ok(self
            .replay_submission_with_target_for_job(job_id)?
            .map(|(_, submission)| submission))
    }

    pub fn replay_submission_with_target_for_job(
        &self,
        job_id: &str,
    ) -> Result<Option<(String, Submission)>, QueueRunnerError> {
        Ok(list_filtered(&self.db_path, None, None)?
            .into_iter()
            .find(|job| job.job_id == job_id)
            .map(|job| {
                let target_thread_id = job.target_thread_id.clone();
                (target_thread_id, super::retry::replay_existing(job))
            }))
    }
}
