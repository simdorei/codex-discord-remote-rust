use super::confirmation::recovery_error;
use super::response::{prepare_component_response, submit_component_response};
use super::{
    ClaimFailureDisposition, ComponentResponse, ComponentWorkerError, ConfirmationClaimState,
    ConfirmationPlan, busy, claim_standard_action, now, record_confirmation_ready,
    retain_or_release_component_claim, standard_confirmation_plan, standard_ready_marker,
};
use crate::{
    action_executor::ActionExecutor, discord_dispatch::InboundInteractionWork,
    queue_runner::TurnBackend,
};
use cdr_app_server::ResidentAppServer;
use cdr_discord::components::{ComponentId, persistent_component_claim_key};
use cdr_store::claims::release_component_claim;

use super::COMPONENT_CLAIM_TTL_SECONDS;

pub(super) async fn prepare_component_action<B: TurnBackend>(
    work: &InboundInteractionWork,
    component: &ComponentId,
    executor: &ActionExecutor<B>,
    server: &ResidentAppServer,
) -> Result<ConfirmationPlan, ComponentWorkerError> {
    if matches!(component, ComponentId::Busy { .. }) {
        return Ok(busy::handle_busy_component(work, component, executor, server).await?);
    }
    let mirror_db = executor.mirror_db();
    let message_id = work
        .source_message_id
        .ok_or(ComponentWorkerError::MissingSourceMessage)?;
    if matches!(
        component,
        ComponentId::Approval { .. } | ComponentId::Input { .. }
    ) {
        return Err(ComponentWorkerError::LegacyComponentExpired);
    }
    let claim_id = format!(
        "actor-v1:{}:{}:{}",
        claim_id(message_id.get(), component)?,
        work.user_id,
        work.channel_id
    );
    let plan = standard_confirmation_plan(component, &claim_id)?;
    let ready_marker = standard_ready_marker(&claim_id);
    match claim_standard_action(
        mirror_db,
        &claim_id,
        &ready_marker,
        now()?,
        COMPONENT_CLAIM_TTL_SECONDS,
    )? {
        ConfirmationClaimState::DeliverConfirmation => {
            return Ok(plan);
        }
        ConfirmationClaimState::ActionUnconfirmed => {
            return Err(ComponentWorkerError::ActionUnconfirmed);
        }
        ConfirmationClaimState::ExecuteAction => {}
    }
    let response = match prepare_component_response(component, server).await {
        Ok(response) => response,
        Err(error) => {
            let _ = release_component_claim(mirror_db, &claim_id)?;
            return Err(error);
        }
    };
    let authorized = authorize_response(work, &response, executor, server).await;
    if let Err(error) = authorized {
        let _ = release_component_claim(mirror_db, &claim_id)?;
        return Err(error);
    }
    if let Err(error) = submit_component_response(response, server).await {
        if retain_or_release_component_claim(mirror_db, &claim_id, &error)?
            == ClaimFailureDisposition::RetainIndeterminate
        {
            return Err(ComponentWorkerError::ActionOutcomeIndeterminate(
                error.to_string(),
            ));
        }
        return Err(error.into());
    }
    let completed_at = now().map_err(|error| recovery_error(&error))?;
    let _ = record_confirmation_ready(
        mirror_db,
        &ready_marker,
        completed_at,
        COMPONENT_CLAIM_TTL_SECONDS,
    )
    .map_err(|error| recovery_error(&error))?;
    Ok(plan)
}

fn claim_id(message_id: u64, component: &ComponentId) -> Result<String, ComponentWorkerError> {
    persistent_component_claim_key(message_id, component)
        .ok_or(ComponentWorkerError::InvalidComponent)
}

async fn authorize_response<B: TurnBackend>(
    work: &InboundInteractionWork,
    response: &ComponentResponse,
    executor: &ActionExecutor<B>,
    server: &ResidentAppServer,
) -> Result<(), ComponentWorkerError> {
    let requests = server.pending_server_requests(None).await?;
    let request = requests
        .iter()
        .find(|request| {
            request.id == response.request_id && request.occurrence == response.occurrence
        })
        .ok_or(ComponentWorkerError::NoPendingRequest)?;
    let authority = crate::server_prompt_authority::verify(
        executor.mirror_db(),
        server,
        request,
        response.generation,
    )
    .await?;
    authority.require_actor(work.channel_id.get(), work.user_id.get())?;
    Ok(())
}
