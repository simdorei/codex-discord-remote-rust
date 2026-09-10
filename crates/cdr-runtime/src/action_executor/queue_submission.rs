use cdr_store::StoreError;
use cdr_store::prompt_intake::{PromptIntakeClaim, canonicalize_prompt_intake_target};

use super::app_server_target::ActionTarget;
use super::queue_result::submission_result;
use super::{ActionError, ActionExecutor, ActionResult, id_i64};
use crate::queue_runner::{QueueRunnerError, Submission, TurnBackend};

pub(super) struct PreparedPromptSubmission<'a> {
    pub channel_id: u64,
    pub user_id: u64,
    pub discord_message_id: Option<u64>,
    pub auto_queue_when_busy: bool,
    pub intake_claim: Option<&'a PromptIntakeClaim>,
    pub raw_prompt: &'a str,
}

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn submit_prepared_target(
        &self,
        mut target: ActionTarget,
        request: PreparedPromptSubmission<'_>,
    ) -> Result<ActionResult, ActionError> {
        for target_attempt in 0..=1 {
            let busy = self.queue.busy_status(&target.thread_id).await?;
            if busy.busy && !request.auto_queue_when_busy {
                return self
                    .busy_result(
                        &target.thread_id,
                        request.channel_id,
                        request.user_id,
                        request.raw_prompt,
                        busy.allow_steer,
                        target.mirror_mapping,
                    )
                    .await;
            }
            let prompt = self
                .prepare_prompt(request.raw_prompt, &target.thread_id)
                .await?;
            let submitted = self
                .submit_to_target(
                    &target,
                    request.channel_id,
                    request.user_id,
                    request.discord_message_id,
                    request.intake_claim,
                    &prompt,
                )
                .await;
            match submitted {
                Ok(submission) => {
                    let (target, submission) = self
                        .recover_active_writer_submission(target, submission)
                        .await?;
                    return Ok(submission_result(
                        &target.thread_id,
                        Some(&target.source_label),
                        &submission,
                        request.raw_prompt,
                    ));
                }
                Err(error) if target.mirror_mapping && is_mapping_change(&error) => {
                    if !self.queue.backend.requires_app_server_fork() {
                        return Err(ActionError::Invalid(format!(
                            "original mirror mapping changed; request preserved without retargeting: {error}"
                        )));
                    }
                    if target_attempt == 1 {
                        return Err(ActionError::Invalid(format!(
                            "mirror mapping changed again while queueing; no request was queued: {error}"
                        )));
                    }
                    let _ = self.canonicalize_completed_target(&target.thread_id)?;
                    let (current, source) =
                        self.current_mirror_target(id_i64(request.channel_id)?, &target.thread_id)?;
                    if source != "mirror" {
                        return Err(ActionError::Invalid(format!(
                            "mirror mapping disappeared while queueing; no request was queued: {error}"
                        )));
                    }
                    target = self.prepare_action_target(&current, source).await?;
                }
                Err(error) if is_completed_source_move(&error) => {
                    if target_attempt == 1 {
                        return Err(ActionError::Invalid(format!(
                            "action target changed again while queueing; no request was queued: {error}"
                        )));
                    }
                    let mirror_mapping = target.mirror_mapping;
                    target = self
                        .prepare_action_target(&target.thread_id, &target.source_label)
                        .await?;
                    target.mirror_mapping = mirror_mapping;
                }
                Err(error) => return Err(error.into()),
            }
        }
        unreachable!("mapping retry loop always returns")
    }

    async fn submit_to_target(
        &self,
        target: &ActionTarget,
        channel_id: u64,
        user_id: u64,
        discord_message_id: Option<u64>,
        intake_claim: Option<&PromptIntakeClaim>,
        prompt: &str,
    ) -> Result<Submission, QueueRunnerError> {
        if let Some(claim) = intake_claim {
            let current = canonicalize_prompt_intake_target(&self.mirror_db, &claim.intake.job_id)?
                .ok_or_else(|| StoreError::PromptIntakeClaimLost {
                    job_id: claim.intake.job_id.clone(),
                })?;
            let refreshed = PromptIntakeClaim {
                intake: current,
                claim_token: claim.claim_token.clone(),
            };
            return self
                .queue
                .submit_prompt_intake(&refreshed, &target.thread_id, prompt)
                .await;
        }
        if target.mirror_mapping {
            self.queue
                .submit_mirror(
                    &target.thread_id,
                    channel_id,
                    user_id,
                    discord_message_id,
                    prompt,
                )
                .await
        } else {
            self.queue
                .submit(
                    &target.thread_id,
                    channel_id,
                    user_id,
                    discord_message_id,
                    prompt,
                )
                .await
        }
    }
}

fn is_mapping_change(error: &QueueRunnerError) -> bool {
    matches!(
        error,
        QueueRunnerError::Store(StoreError::MirrorMappingChanged { .. })
    )
}

fn is_completed_source_move(error: &QueueRunnerError) -> bool {
    matches!(
        error,
        QueueRunnerError::Store(StoreError::ForkHandoffTargetMoved { .. })
    )
}
