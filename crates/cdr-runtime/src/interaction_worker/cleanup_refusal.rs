use super::{ExecutionCustody, InteractionWorkerError};
use crate::discord_dispatch::InboundInteractionWork;
use cdr_discord::http::DiscordHttp;
use cdr_store::ingress::CleanupRefusal;

pub(super) async fn deliver(
    work: &InboundInteractionWork,
    custody: &mut ExecutionCustody,
    api: &DiscordHttp,
    refusal: &CleanupRefusal,
) -> Result<bool, InteractionWorkerError> {
    // This updates custody's flag as well as the durable outcome. Normal finish
    // can confirm the notification, but cannot replace sync_completed=false.
    custody.record_result(&refusal.outcome())?;
    // The bounded schema uses only a room id and a fixed protection reason.
    // Exactly one PATCH to the existing deferred reply; never a follow-up POST.
    api.update_initial_response(&work.interaction_token, &refusal.message())
        .await
        .map_err(|error| {
            crate::cleanup_refusal::NotificationFailure::delivery(
                &work.custody_database,
                &work.custody_ingress_id,
                error,
            )
        })?;
    Ok(false)
}
