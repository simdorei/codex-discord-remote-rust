use cdr_discord::responses::interaction_message;
use tokio::time::{Instant, sleep_until};
use twilight_model::http::interaction::InteractionResponse;
use twilight_model::id::{Id, marker::InteractionMarker};

use super::claims::InteractionClaim;
use super::custody::StagedCustody;
use super::{DiscordDispatchError, DispatchOutcome, InteractionDispatcher, InteractionTransport};

impl<T: InteractionTransport> InteractionDispatcher<T> {
    pub(super) async fn reject_missing_identity(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        deadline: Instant,
        claim: InteractionClaim,
    ) -> Result<DispatchOutcome, DiscordDispatchError> {
        self.respond_without_work(
            interaction_id,
            token,
            deadline,
            claim,
            "Discord interaction identity is missing.",
        )
        .await
    }

    pub(super) async fn reject_busy_choice_unavailable(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        deadline: Instant,
        claim: InteractionClaim,
    ) -> Result<DispatchOutcome, DiscordDispatchError> {
        self.respond_without_work(
            interaction_id,
            token,
            deadline,
            claim,
            "This busy-choice button is no longer active. Please use the latest prompt.",
        )
        .await
    }

    async fn respond_without_work(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        deadline: Instant,
        claim: InteractionClaim,
        content: &str,
    ) -> Result<DispatchOutcome, DiscordDispatchError> {
        let response = interaction_message(content, true);
        if !self
            .acknowledge_until(interaction_id, token, &response, deadline)
            .await?
        {
            return Ok(DispatchOutcome::DeadlineExceeded);
        }
        commit_claim(claim)?;
        Ok(DispatchOutcome::RespondedWithoutWork)
    }

    pub(super) async fn acknowledge_until(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        response: &InteractionResponse,
        deadline: Instant,
    ) -> Result<bool, DiscordDispatchError> {
        let mut acknowledgement = self.transport.acknowledge(interaction_id, token, response);
        tokio::select! {
            biased;
            () = sleep_until(deadline) => Ok(false),
            result = acknowledgement.as_mut() => {
                result.map(|()| true).map_err(DiscordDispatchError::Acknowledge)
            },
        }
    }

    pub(super) async fn reject_unavailable(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        deadline: Instant,
        claim: InteractionClaim,
        mut custody: StagedCustody,
        outcome: DispatchOutcome,
    ) -> Result<DispatchOutcome, DiscordDispatchError> {
        let (reason, content) = match outcome {
            DispatchOutcome::QueueFull => (
                "interaction_queue_full",
                "Codex Discord work queue is full. Please retry shortly.",
            ),
            _ => (
                "interaction_queue_closed",
                "Codex Discord runtime is stopping. Please retry after restart.",
            ),
        };
        custody
            .hold_not_executed(reason)
            .map_err(|error| custody_error(&error))?;
        let response = interaction_message(content, true);
        if !self
            .acknowledge_until(interaction_id, token, &response, deadline)
            .await?
        {
            return Ok(DispatchOutcome::DeadlineExceeded);
        }
        commit_claim(claim)?;
        Ok(outcome)
    }
}

pub(super) fn custody_error(error: &cdr_store::StoreError) -> DiscordDispatchError {
    DiscordDispatchError::Custody(error.to_string())
}

pub(super) fn hold_or_log(custody: &mut StagedCustody, reason: &'static str) {
    if let Err(error) = custody.hold_not_executed(reason) {
        eprintln!("interaction_custody_hold_failed phase={reason} error={error}");
    }
}

pub(super) fn commit_claim(claim: InteractionClaim) -> Result<(), DiscordDispatchError> {
    if claim.commit() {
        Ok(())
    } else {
        Err(DiscordDispatchError::ClaimState)
    }
}
