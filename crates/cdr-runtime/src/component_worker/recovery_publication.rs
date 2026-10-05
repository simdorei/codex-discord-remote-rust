//! A dedicated intent sink. No server, backend or queue-start API is called.
use std::time::Duration;

use cdr_discord::{components::ComponentId, interaction::RoutedWork};
use cdr_store::{StoreError, async_resolution::publication as store};

use super::{ComponentWorkerError, ConfirmationPlan};
use crate::{
    action_executor::ActionExecutor,
    discord_dispatch::{InboundInteractionWork, InteractionProcessingMode},
    queue_runner::TurnBackend,
};

pub(super) async fn handle<B: TurnBackend>(
    work: &InboundInteractionWork,
    component: &ComponentId,
    executor: &ActionExecutor<B>,
) -> Result<ConfirmationPlan, ComponentWorkerError> {
    let ComponentId::RecoveryPublicationDecision {
        proposal_id,
        revision,
        ..
    } = component
    else {
        return Err(ComponentWorkerError::InvalidComponent);
    };
    // The real dispatcher keeps this permit alive through worker completion.
    // It is maintenance admission, not a publication execution permit.
    if work.admission_permit.is_none()
        || work.processing_mode != InteractionProcessingMode::Execute
        || work.work != RoutedWork::Component(component.clone())
    {
        return Err(invalid("publication intent has no matching live admission"));
    }
    let db = executor.mirror_db();
    if std::fs::canonicalize(db).map_err(StoreError::from)?
        != std::fs::canonicalize(&work.custody_database).map_err(StoreError::from)?
    {
        return Err(invalid("publication intent custody database changed"));
    }
    let before = authorized_delivery(db, work, proposal_id, *revision)?;
    let _lock = tokio::time::timeout(
        Duration::from_secs(2),
        executor.control_lock(&before.proposal.thread_id),
    )
    .await
    .map_err(|_| invalid("publication intent target is busy; nothing was started"))?
    .map_err(|error| invalid(&error.to_string()))?;
    let current = authorized_delivery(db, work, proposal_id, *revision)?;
    if current != before {
        return Err(invalid(
            "publication delivery changed while waiting for target lock",
        ));
    }
    let ingress = cdr_store::ingress::get(db, &work.custody_ingress_id)?
        .ok_or_else(|| invalid("publication intent ingress is missing"))?;
    if ingress.event_id != Some(sql_id(work.interaction_id.get())?)
        || ingress.application_id != Some(sql_id(work.application_id.get())?)
        || ingress.channel_id != sql_id(work.channel_id.get())?
        || ingress.owner_user_id != sql_id(work.user_id.get())?
        || ingress.source_message_id
            != work
                .source_message_id
                .map(|id| sql_id(id.get()))
                .transpose()?
        || ingress.target_thread_id.as_deref() != Some(current.proposal.thread_id.as_str())
        || ingress.payload["work"] != serde_json::to_value(&work.work).map_err(StoreError::from)?
    {
        return Err(invalid(
            "publication intent differs from original durable ingress",
        ));
    }
    let receipt = store::record_consent(
        db,
        &store::ConsentInput {
            proposal_id,
            revision: *revision,
            ingress_id: &work.custody_ingress_id,
            now: super::now()?,
        },
    )?;
    Ok(ConfirmationPlan {
        content: match receipt.decision {
            store::Decision::ApproveExact => {
                "Exact recovery intent recorded. No request was started; separate safety checks are still required."
            }
            store::Decision::KeepHeld => "Recovery will remain held. No request was started.",
        },
        domain: "recovery-publication-intent-confirmation-v1",
        logical_key: format!("{proposal_id}:{revision}"),
    })
}

fn authorized_delivery(
    db: &std::path::Path,
    work: &InboundInteractionWork,
    id: &str,
    revision: i64,
) -> Result<store::DeliveredProposal, ComponentWorkerError> {
    let binding = store::delivered_proposal(db, id, revision)?;
    binding.require_actor(
        sql_id(work.application_id.get())?,
        sql_id(work.channel_id.get())?,
        sql_id(work.user_id.get())?,
        sql_id(
            work.source_message_id
                .ok_or(ComponentWorkerError::MissingSourceMessage)?
                .get(),
        )?,
    )?;
    Ok(binding)
}

fn sql_id(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Integrity("Discord ID exceeds SQLite range".into()))
}

fn invalid(reason: &str) -> ComponentWorkerError {
    ComponentWorkerError::PublicationConsent(reason.into())
}
