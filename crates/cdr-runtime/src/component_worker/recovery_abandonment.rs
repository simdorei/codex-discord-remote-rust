//! Dedicated local disposition. It cannot call a native approval or start API.
use super::{ComponentWorkerError, ConfirmationPlan};
use crate::{
    action_executor::ActionExecutor,
    discord_dispatch::{InboundInteractionWork, InteractionProcessingMode},
    queue_runner::TurnBackend,
};
use cdr_discord::{
    components::{AbandonDecision, ComponentId},
    interaction::RoutedWork,
};
use cdr_store::{StoreError, async_resolution::abandonment as store};
use std::{path::Path, time::Duration};

pub(super) const CONFIRMATION_DOMAIN: &str = "recovery-abandonment-confirmation-v1";

pub(super) async fn handle<B: TurnBackend>(
    work: &InboundInteractionWork,
    component: &ComponentId,
    executor: &ActionExecutor<B>,
) -> Result<ConfirmationPlan, ComponentWorkerError> {
    let ComponentId::RecoveryAbandonDecision {
        proposal_id,
        revision,
        decision,
    } = component
    else {
        return Err(ComponentWorkerError::InvalidComponent);
    };
    if work.admission_permit.is_none()
        || work.processing_mode != InteractionProcessingMode::Execute
        || work.work != RoutedWork::Component(component.clone())
        || work.custody_ingress_id != format!("interaction:{}", work.interaction_id)
    {
        return Err(invalid("no matching live normal admission"));
    }
    let db = executor.mirror_db();
    if std::fs::canonicalize(db).map_err(StoreError::from)?
        != std::fs::canonicalize(&work.custody_database).map_err(StoreError::from)?
    {
        return Err(invalid("custody database changed"));
    }
    let decision = match decision {
        AbandonDecision::AbandonOnly => store::Decision::AbandonOnly,
        AbandonDecision::KeepHeld => store::Decision::KeepHeld,
    };
    let before = authorize(db, work, proposal_id, *revision, decision)?;
    let _lock = tokio::time::timeout(
        Duration::from_secs(2),
        executor.control_lock(&before.proposal.thread_id),
    )
    .await
    .map_err(|_| invalid("target is busy; no disposition was applied"))?
    .map_err(|e| invalid(&e.to_string()))?;
    let current = authorize(db, work, proposal_id, *revision, decision)?;
    if current != before {
        return Err(invalid("displayed proposal changed while waiting"));
    }
    let saved = cdr_store::ingress::get(db, &work.custody_ingress_id)?
        .ok_or_else(|| invalid("original ingress is unavailable"))?;
    if saved.event_id != Some(sql_id(work.interaction_id.get())?)
        || saved.application_id != Some(sql_id(work.application_id.get())?)
        || saved.channel_id != sql_id(work.channel_id.get())?
        || saved.owner_user_id != sql_id(work.user_id.get())?
        || saved.source_message_id
            != work
                .source_message_id
                .map(|v| sql_id(v.get()))
                .transpose()?
        || saved.target_thread_id.as_deref() != Some(current.proposal.thread_id.as_str())
        || saved.payload["work"] != serde_json::to_value(&work.work).map_err(StoreError::from)?
    {
        return Err(invalid("work differs from original durable ingress"));
    }
    let receipt = store::record_decision(
        db,
        &store::DecisionInput {
            proposal_id,
            revision: *revision,
            ingress_id: &work.custody_ingress_id,
            decision,
            now: super::now()?,
        },
    )?;
    Ok(ConfirmationPlan {
        content: match receipt.decision {
            store::Decision::AbandonOnly => {
                "Saved request permanently abandoned without replay. The thread remains held; no new request was started."
            }
            store::Decision::KeepHeld => {
                "Saved request kept. The thread remains held; no request was replayed or started."
            }
        },
        domain: CONFIRMATION_DOMAIN,
        logical_key: format!("{proposal_id}:{revision}"),
    })
}

fn authorize(
    db: &Path,
    work: &InboundInteractionWork,
    id: &str,
    revision: i64,
    decision: store::Decision,
) -> Result<store::DeliveredProposal, ComponentWorkerError> {
    Ok(store::authorize_decision(
        db,
        &store::DecisionRouteInput {
            proposal_id: id,
            revision,
            decision,
            interaction_id: sql_id(work.interaction_id.get())?,
            application_id: sql_id(work.application_id.get())?,
            channel_id: sql_id(work.channel_id.get())?,
            owner_user_id: sql_id(work.user_id.get())?,
            source_message_id: sql_id(
                work.source_message_id
                    .ok_or(ComponentWorkerError::MissingSourceMessage)?
                    .get(),
            )?,
            now: super::now()?,
        },
    )?)
}
fn sql_id(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Integrity("Discord ID exceeds SQLite range".into()))
}
fn invalid(reason: &str) -> ComponentWorkerError {
    ComponentWorkerError::Abandonment(reason.into())
}
