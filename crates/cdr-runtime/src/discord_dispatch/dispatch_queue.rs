use cdr_discord::interaction::RoutedWork;
use tokio::sync::mpsc;
use tokio::time::Instant;
use twilight_model::http::interaction::InteractionResponse;
use twilight_model::id::{
    Id,
    marker::{ApplicationMarker, ChannelMarker, InteractionMarker, MessageMarker, UserMarker},
};

use super::claims::InteractionClaim;
use super::custody::StagedCustody;
use super::dispatch_response::{custody_error, hold_or_log};
use super::{
    DiscordDispatchError, DispatchOutcome, InboundInteractionWork, InteractionDispatcher,
    InteractionProcessingMode, InteractionTransport,
};
use crate::restart_readiness::drain::AdmissionPermit;

pub(super) struct StagedInteraction {
    pub application_id: Id<ApplicationMarker>,
    pub interaction_id: Id<InteractionMarker>,
    pub channel_id: Id<ChannelMarker>,
    pub user_id: Id<UserMarker>,
    pub source_message_id: Option<Id<MessageMarker>>,
    pub token: String,
    pub work: RoutedWork,
    pub claim: InteractionClaim,
    pub custody: StagedCustody,
    pub admission_permit: Option<AdmissionPermit>,
}

impl<T: InteractionTransport> InteractionDispatcher<T> {
    pub(super) async fn queue_staged(
        &self,
        staged: StagedInteraction,
        response: InteractionResponse,
        deadline: Instant,
    ) -> Result<DispatchOutcome, DiscordDispatchError> {
        let StagedInteraction {
            application_id,
            interaction_id,
            channel_id,
            user_id,
            source_message_id,
            token,
            work,
            claim,
            mut custody,
            admission_permit,
        } = staged;
        let reservation = match self.work.clone().try_reserve_owned() {
            Ok(reservation) => reservation,
            Err(mpsc::error::TrySendError::Full(_)) => {
                return self
                    .reject_unavailable(
                        interaction_id,
                        &token,
                        deadline,
                        claim,
                        custody,
                        DispatchOutcome::QueueFull,
                    )
                    .await;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return self
                    .reject_unavailable(
                        interaction_id,
                        &token,
                        deadline,
                        claim,
                        custody,
                        DispatchOutcome::Stopping,
                    )
                    .await;
            }
        };
        match self
            .acknowledge_until(interaction_id, &token, &response, deadline)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                hold_or_log(&mut custody, "discord_ack_deadline");
                return Ok(DispatchOutcome::DeadlineExceeded);
            }
            Err(error) => {
                hold_or_log(&mut custody, "discord_ack_failed");
                return Err(error);
            }
        }
        if let Err(error) = custody.acknowledge() {
            hold_or_log(&mut custody, "custody_acknowledge_failed");
            return Err(custody_error(&error));
        }
        if !claim.commit() {
            hold_or_log(&mut custody, "interaction_claim_commit_failed");
            return Err(DiscordDispatchError::ClaimState);
        }
        let custody = custody.into_receipt();
        reservation.send(InboundInteractionWork {
            application_id,
            interaction_id,
            channel_id,
            user_id,
            source_message_id,
            interaction_token: token,
            work,
            processing_mode: InteractionProcessingMode::Execute,
            custody_database: custody.database,
            custody_ingress_id: custody.ingress_id,
            authorized_busy_choice: custody.busy_choice,
            admission_permit,
        });
        Ok(DispatchOutcome::Queued)
    }
}
