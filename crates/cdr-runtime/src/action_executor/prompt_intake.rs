mod recovery;

use std::time::{SystemTime, UNIX_EPOCH};

use cdr_store::prompt_intake::{
    NewPromptIntake, PromptIntakeClaim, StoredPromptIntake, admit_prompt_intake,
    canonicalize_prompt_intake_target, get_prompt_intake, record_prompt_intake_failure_if_claimed,
    remove_prompt_intake_if_queued, try_claim_prompt_intake,
};
use uuid::Uuid;

use super::queue_result::submission_result;
use super::queue_submission::PreparedPromptSubmission;
use super::{ActionError, ActionExecutor, ActionResult, id_i64};
use crate::queue_runner::{TurnBackend, retry_delay_seconds};

use self::recovery::ClaimProcessingOutcome;

const INTAKE_CLAIM_SECONDS: f64 = 600.0;
// The stored flag records the caller's original policy. Once admission succeeds,
// the prompt must queue if the target becomes busy instead of requiring a new UI choice.
const QUEUE_AFTER_DURABLE_ADMISSION: bool = true;

pub(super) struct PromptAdmission<'a> {
    pub target_thread_id: &'a str,
    pub source: &'a str,
    pub channel_id: u64,
    pub user_id: u64,
    pub discord_message_id: Option<u64>,
    pub auto_queue_when_busy: bool,
    pub raw_prompt: &'a str,
}

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn admit_prompt(
        &self,
        request: PromptAdmission<'_>,
    ) -> Result<ActionResult, ActionError> {
        let generated_job_id = Uuid::new_v4().to_string();
        let admitted = admit_prompt_intake(
            &self.mirror_db,
            NewPromptIntake {
                job_id: &generated_job_id,
                target_thread_id: request.target_thread_id,
                channel_id: id_i64(request.channel_id)?,
                owner_user_id: Some(id_i64(request.user_id)?),
                discord_message_id: request.discord_message_id.map(id_i64).transpose()?,
                raw_prompt: request.raw_prompt,
                auto_queue_when_busy: request.auto_queue_when_busy,
                require_current_mirror: request.source == "mirror",
                created_at: unix_now()?,
            },
        )?;
        self.process_admitted_prompt(&admitted.intake).await
    }

    pub(super) async fn process_admitted_prompt(
        &self,
        intake: &StoredPromptIntake,
    ) -> Result<ActionResult, ActionError> {
        let Some(claim) = self.try_claim_intake(intake)? else {
            return self.replay_or_pending(intake);
        };
        match self.process_claim_with_renewal(&claim).await {
            ClaimProcessingOutcome::Finished(result) => self.finish_live_claim(&claim, result),
            ClaimProcessingOutcome::Lost => self.replay_or_pending(intake),
        }
    }

    async fn process_claim(&self, claim: &PromptIntakeClaim) -> Result<ActionResult, ActionError> {
        let intake = canonicalize_prompt_intake_target(&self.mirror_db, &claim.intake.job_id)?
            .ok_or_else(|| missing_intake(&claim.intake.job_id))?;
        let (thread_id, source) = self.intake_target(&intake)?;
        let target = self.prepare_action_target(&thread_id, source).await?;
        self.submit_prepared_target(
            target,
            PreparedPromptSubmission {
                channel_id: u64_id(intake.channel_id)?,
                user_id: u64_id(intake.owner_user_id.ok_or_else(|| {
                    ActionError::Invalid(format!(
                        "prompt intake {} has no owner user id",
                        intake.job_id
                    ))
                })?)?,
                discord_message_id: intake.discord_message_id.map(u64_id).transpose()?,
                auto_queue_when_busy: QUEUE_AFTER_DURABLE_ADMISSION,
                intake_claim: Some(claim),
                raw_prompt: &intake.raw_prompt,
            },
        )
        .await
    }

    fn intake_target<'a>(
        &self,
        intake: &'a StoredPromptIntake,
    ) -> Result<(String, &'a str), ActionError> {
        if !intake.require_current_mirror {
            return Ok((intake.target_thread_id.clone(), "selected"));
        }
        let (target, source) =
            self.current_mirror_target(intake.channel_id, &intake.target_thread_id)?;
        if source != "mirror"
            || (!self.queue.backend.requires_app_server_fork() && target != intake.target_thread_id)
        {
            return Err(ActionError::Invalid(format!(
                "prompt intake {} lost or changed its original mirror mapping; it remains saved for recovery",
                intake.job_id
            )));
        }
        Ok((target, source))
    }

    fn finish_live_claim(
        &self,
        claim: &PromptIntakeClaim,
        result: Result<ActionResult, ActionError>,
    ) -> Result<ActionResult, ActionError> {
        match result {
            Ok(result) => {
                self.require_atomic_promotion(&claim.intake.job_id)?;
                Ok(result)
            }
            Err(error) => {
                if cdr_store::dead_generation::target_is_held(
                    &self.mirror_db,
                    &claim.intake.target_thread_id,
                )? || matches!(
                    &error,
                    ActionError::Store(cdr_store::StoreError::DeadGenerationTargetHeld(_))
                        | ActionError::Queue(crate::queue_runner::QueueRunnerError::Store(
                            cdr_store::StoreError::DeadGenerationTargetHeld(_)
                        ))
                ) {
                    return Err(ActionError::Invalid(format!(
                        "request {} is preserved under a manual hold and will not be retried automatically: {error}",
                        claim.intake.job_id
                    )));
                }
                if remove_prompt_intake_if_queued(&self.mirror_db, &claim.intake.job_id)? {
                    return Err(error);
                }
                if get_prompt_intake(&self.mirror_db, &claim.intake.job_id)?.is_none() {
                    return Err(error);
                }
                self.backoff_claim(claim, &error)?;
                Err(ActionError::Invalid(format!(
                    "request {} is saved for automatic recovery and was not started: {error}",
                    claim.intake.job_id
                )))
            }
        }
    }

    fn replay_or_pending(&self, intake: &StoredPromptIntake) -> Result<ActionResult, ActionError> {
        if let Some((target, submission)) = self
            .queue
            .replay_submission_with_target_for_job(&intake.job_id)?
        {
            return Ok(submission_result(
                &target,
                Some(intake_source(intake)),
                &submission,
                &intake.raw_prompt,
            ));
        }
        let current = canonicalize_prompt_intake_target(&self.mirror_db, &intake.job_id)?;
        let Some(current) = current else {
            if let Some((target, submission)) = self
                .queue
                .replay_submission_with_target_for_job(&intake.job_id)?
            {
                return Ok(submission_result(
                    &target,
                    Some(intake_source(intake)),
                    &submission,
                    &intake.raw_prompt,
                ));
            }
            return Err(missing_intake(&intake.job_id));
        };
        Ok(ActionResult {
            text: format!(
                "Accepted Codex request; durable preparation or retry is already pending\nthread_id: {}\njob_id: {}",
                current.target_thread_id, current.job_id
            ),
            waits_for_final: true,
            ui: None,
        })
    }

    fn try_claim_intake(
        &self,
        intake: &StoredPromptIntake,
    ) -> Result<Option<PromptIntakeClaim>, ActionError> {
        self.try_claim_intake_at(intake, unix_now()?)
    }

    fn try_claim_intake_at(
        &self,
        intake: &StoredPromptIntake,
        now: f64,
    ) -> Result<Option<PromptIntakeClaim>, ActionError> {
        Ok(try_claim_prompt_intake(
            &self.mirror_db,
            &intake.job_id,
            now,
            now + INTAKE_CLAIM_SECONDS,
        )?)
    }

    fn backoff_claim(
        &self,
        claim: &PromptIntakeClaim,
        error: &ActionError,
    ) -> Result<(), ActionError> {
        let now = unix_now()?;
        let next_attempt = claim.intake.attempt_count.saturating_add(1);
        let retry_after = now + f64::from(retry_delay_seconds(next_attempt));
        if record_prompt_intake_failure_if_claimed(
            &self.mirror_db,
            claim,
            &error.to_string(),
            retry_after,
        )?
        .is_none()
        {
            return Err(ActionError::Invalid(format!(
                "prompt intake {} processing failed and its durable claim changed: {error}",
                claim.intake.job_id
            )));
        }
        Ok(())
    }

    fn require_atomic_promotion(&self, job_id: &str) -> Result<(), ActionError> {
        if get_prompt_intake(&self.mirror_db, job_id)?.is_none() {
            Ok(())
        } else {
            Err(ActionError::Invalid(format!(
                "prompt intake {job_id} was not atomically promoted to the durable queue"
            )))
        }
    }
}

fn unix_now() -> Result<f64, ActionError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

fn u64_id(value: i64) -> Result<u64, ActionError> {
    u64::try_from(value).map_err(|_| ActionError::IntegerRange)
}

fn missing_intake(job_id: &str) -> ActionError {
    ActionError::Invalid(format!(
        "prompt intake disappeared before processing: {job_id}"
    ))
}

fn intake_source(intake: &StoredPromptIntake) -> &'static str {
    if intake.require_current_mirror {
        "mirror"
    } else {
        "selected"
    }
}
