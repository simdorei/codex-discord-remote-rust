use cdr_store::prompt_intake::PromptIntakeClaim;

use super::submission::SubmissionRequest;
use super::{QueueCoordinator, QueueRunnerError, Submission, TurnBackend};

impl<B: TurnBackend> QueueCoordinator<B> {
    pub async fn submit_prompt_intake(
        &self,
        claim: &PromptIntakeClaim,
        target_thread_id: &str,
        prompt: &str,
    ) -> Result<Submission, QueueRunnerError> {
        self.submit_with_mapping(SubmissionRequest {
            job_id: &claim.intake.job_id,
            target_thread_id,
            channel_id: u64::try_from(claim.intake.channel_id)
                .map_err(|_| QueueRunnerError::IntegerRange)?,
            owner_user_id: u64::try_from(
                claim
                    .intake
                    .owner_user_id
                    .ok_or(QueueRunnerError::IntegerRange)?,
            )
            .map_err(|_| QueueRunnerError::IntegerRange)?,
            discord_message_id: claim
                .intake
                .discord_message_id
                .map(u64::try_from)
                .transpose()
                .map_err(|_| QueueRunnerError::IntegerRange)?,
            prompt,
            require_current_mirror: claim.intake.require_current_mirror,
            intake_claim: Some(claim),
        })
        .await
    }
}
