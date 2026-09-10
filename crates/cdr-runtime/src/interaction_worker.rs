use std::sync::Arc;

use cdr_app_server::ResidentAppServer;
use cdr_discord::components::ComponentError;
use cdr_discord::delivery::DeliveryFailure;
use cdr_discord::http::{DiscordHttp, DiscordHttpError};
use cdr_discord::interaction::RoutedWork;
use thiserror::Error;
use tokio::sync::mpsc;
use twilight_http::Client;

use crate::action_executor::{ActionContext, ActionError, ActionExecutor};
use crate::command_plan::{CommandPlanError, plan_slash};
use crate::component_worker::{ComponentWorkerError, handle_component_work};
use crate::discord_dispatch::InboundInteractionWork;
use crate::discord_dispatch::delivery_identity::{
    COMPONENT_ERROR_DOMAIN, INTERACTION_ERROR_DOMAIN, INTERACTION_FOLLOWUP_DOMAIN,
    component_claim_identity, component_delivery_key,
};
use crate::queue_runner::TurnBackend;

mod action_delivery;
#[cfg(test)]
mod busy_ingress_tests;
mod custody;
mod delivery;
pub mod error_disposition;
mod new_reply;

#[cfg(test)]
#[path = "interaction_worker/connected_tests.rs"]
mod connected_tests;
#[cfg(test)]
mod error_receipt_tests;

use custody::ExecutionCustody;
use delivery::deliver_interaction_text_idempotent;
use error_disposition::{InteractionErrorDisposition, interaction_error_disposition};

#[derive(Debug, Error)]
pub enum InteractionWorkerError {
    #[error(transparent)]
    PromptDelivery(#[from] crate::server_prompt_delivery::PromptDeliveryError),
    #[error(transparent)]
    Plan(#[from] CommandPlanError),
    #[error(transparent)]
    Action(#[from] ActionError),
    #[error("Discord interaction delivery failed: {0:?}")]
    Delivery(DeliveryFailure<DiscordHttpError>),
    #[error("interaction work type is not implemented yet: {0}")]
    Unsupported(&'static str),
    #[error(transparent)]
    Component(#[from] ComponentWorkerError),
    #[error(transparent)]
    Ui(#[from] ComponentError),
    #[error("durable interaction custody failed: {0}")]
    Custody(#[from] cdr_store::StoreError),
}

async fn process_interaction_work<B: TurnBackend>(
    work: &InboundInteractionWork,
    executor: &ActionExecutor<B>,
    server: &ResidentAppServer,
    http: Arc<Client>,
    custody: &mut ExecutionCustody,
) -> Result<bool, InteractionWorkerError> {
    if let Some(reason) = custody.request_rejection(work)? {
        custody.record_result(&serde_json::json!({
            "kind":"request_rejected", "action_completed":false, "control_dispatched":false,
            "error":reason,
        }))?;
        let result = crate::action_executor::ActionResult {
            text: format!("ERROR: {reason}"),
            waits_for_final: false,
            ui: None,
        };
        action_delivery::deliver(work, &result, executor.mirror_db(), server, http).await?;
        return Ok(false);
    }
    let result = match &work.work {
        RoutedWork::Slash(invocation) => {
            let action = plan_slash(invocation)?;
            let result = executor
                .execute_with_ingress_context(
                    action,
                    ActionContext {
                        channel_id: work.channel_id.get(),
                        user_id: work.user_id.get(),
                        discord_message_id: Some(work.interaction_id.get()),
                        auto_queue_when_busy: false,
                    },
                    &work.custody_ingress_id,
                )
                .await?;
            custody.record_result(&serde_json::json!({
                "kind": "slash",
                "action_completed": true,
                "waits_for_final": result.waits_for_final,
                "response": result.text,
            }))?;
            result
        }
        RoutedWork::Autocomplete(_) => {
            return Err(InteractionWorkerError::Unsupported("autocomplete"));
        }
        RoutedWork::Component(component) => {
            let confirmation = match handle_component_work(work, component, executor, server).await
            {
                Ok(confirmation) => confirmation,
                Err(
                    error @ ComponentWorkerError::Busy(
                        crate::component_worker::BusyComponentError::ControlNotDispatched(_),
                    ),
                ) => {
                    custody.record_result(&serde_json::json!({
                        "kind": "busy_control_preflight_rejected",
                        "control_dispatched": false,
                        "error": error.to_string(),
                    }))?;
                    return Err(error.into());
                }
                Err(error) if error.action_completed_before_failure() => {
                    custody.record_result(&serde_json::json!({
                        "kind": "component",
                        "action_completed": true,
                        "confirmation_error": error.to_string(),
                    }))?;
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            };
            custody.record_result(&serde_json::json!({
                "kind": "component",
                "action_completed": true,
            }))?;
            confirmation.deliver(http).await?;
            return Ok(false);
        }
    };
    action_delivery::deliver(work, &result, executor.mirror_db(), server, http).await?;
    Ok(result.waits_for_final)
}

pub async fn run_interaction_worker<B: TurnBackend>(
    mut receiver: mpsc::Receiver<InboundInteractionWork>,
    executor: Arc<ActionExecutor<B>>,
    server: Arc<ResidentAppServer>,
    http: Arc<Client>,
) {
    while let Some(work) = receiver.recv().await {
        let mut custody = match ExecutionCustody::begin(
            executor.mirror_db(),
            &work.custody_database,
            &work.custody_ingress_id,
            work.processing_mode,
        ) {
            Ok(custody) => custody,
            Err(error) => {
                report_interaction_error(
                    &work,
                    InteractionWorkerError::Custody(error),
                    Arc::clone(&http),
                )
                .await;
                continue;
            }
        };
        let result =
            process_interaction_work(&work, &executor, &server, Arc::clone(&http), &mut custody)
                .await;
        match result {
            Ok(waits_for_final) => {
                if let Err(error) = custody.finish_success(&serde_json::json!({
                    "kind": "interaction",
                    "action_completed": true,
                    "waits_for_final": waits_for_final,
                })) {
                    report_interaction_error(
                        &work,
                        InteractionWorkerError::Custody(error),
                        Arc::clone(&http),
                    )
                    .await;
                } else {
                    executor.notify_delivery_ready();
                }
            }
            Err(error) => {
                if let Err(hold_error) = custody.hold_failed() {
                    eprintln!(
                        "interaction_processing_hold_failed ingress={} error={hold_error}",
                        work.custody_ingress_id
                    );
                }
                report_interaction_error(&work, error, Arc::clone(&http)).await;
            }
        }
    }
}

async fn report_interaction_error(
    work: &InboundInteractionWork,
    error: InteractionWorkerError,
    http: Arc<Client>,
) {
    match interaction_error_disposition(&error) {
        InteractionErrorDisposition::IgnoreDuplicate => return,
        InteractionErrorDisposition::LogOnly => {
            eprintln!("component_confirmation_recovery_error: {error}");
            return;
        }
        InteractionErrorDisposition::Report => {}
    }
    let error_text = format!("ERROR: {error}");
    if let RoutedWork::Component(component) = &work.work {
        let claim_identity = component_claim_identity(work.source_message_id, component);
        let logical_key = component_delivery_key(
            work.interaction_id,
            work.source_message_id,
            component,
            claim_identity.as_deref(),
        );
        let delivery = crate::completion_worker::send_recorded_message_with_components(
            &work.custody_database,
            &http,
            work.channel_id,
            &crate::completion_worker::IdempotentChunk {
                domain: COMPONENT_ERROR_DOMAIN,
                logical_key,
                chunk_index: 0,
                content: error_text,
            },
            &[],
        )
        .await;
        if let Err(delivery_error) = delivery {
            eprintln!(
                "ERROR: {error}; additionally failed to report to Discord: {delivery_error:?}"
            );
        }
        return;
    }
    let api = DiscordHttp::new(http, work.application_id);
    let delivery =
        deliver_interaction_text_idempotent(&api, work, &error_text, INTERACTION_ERROR_DOMAIN)
            .await;
    if let Err(delivery_error) = delivery {
        eprintln!("ERROR: {error}; additionally failed to report to Discord: {delivery_error:?}");
    }
}
