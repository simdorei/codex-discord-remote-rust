use cdr_discord::gateway::ingress::GatewayIngressReceivers;
use tokio::sync::{oneshot, watch};

use super::super::worker_supervision::{MonitoredWorker, WorkerExitNotifier};
use super::{
    TypedIngressContext, history, interaction, message, ready, receive_error, spawn_announced,
};

pub(super) type ReadyReceiver = (&'static str, oneshot::Receiver<()>);

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_consumers(
    receivers: GatewayIngressReceivers,
    context: TypedIngressContext,
    shutdown: watch::Receiver<bool>,
    ready_identity: cdr_discord::gateway::GatewayIdentityReceiver,
    ready_conflict: cdr_discord::gateway::GatewayIdentityConflictReceiver,
    message_identity: cdr_discord::gateway::GatewayIdentityReceiver,
    message_conflict: cdr_discord::gateway::GatewayIdentityConflictReceiver,
    emergency_identity: (
        cdr_discord::gateway::GatewayIdentityReceiver,
        cdr_discord::gateway::GatewayIdentityConflictReceiver,
    ),
    history_identity: cdr_discord::gateway::GatewayIdentityReceiver,
    history_conflict: cdr_discord::gateway::GatewayIdentityConflictReceiver,
    message_gaps: cdr_discord::gateway::ingress::MessageGapReceiver,
    exit_notifier: &WorkerExitNotifier,
) -> (Vec<MonitoredWorker>, Vec<ReadyReceiver>) {
    let GatewayIngressReceivers {
        normal_interactions,
        reserved_interactions,
        messages,
        emergency_messages,
        receive_errors,
    } = receivers;
    let interaction_handler = interaction::DiscordInteractionHandler::from_context(&context);
    let mut handles = Vec::with_capacity(7);
    let mut readiness = Vec::with_capacity(7);
    macro_rules! spawn {
        ($lane:literal, $future:expr) => {{
            let (sender, receiver) = oneshot::channel();
            handles.push(spawn_announced(
                $lane,
                sender,
                exit_notifier.clone(),
                $future,
            ));
            readiness.push(($lane, receiver));
        }};
    }
    spawn!(
        "ready",
        ready::run(
            ready_identity,
            ready_conflict,
            context.clone(),
            shutdown.clone()
        )
    );
    spawn!(
        "message-emergency",
        message::run(
            emergency_messages,
            emergency_identity.0,
            emergency_identity.1,
            context.clone(),
            shutdown.clone()
        )
    );
    spawn!(
        "message",
        message::run(
            messages,
            message_identity,
            message_conflict,
            context.clone(),
            shutdown.clone()
        )
    );
    spawn!(
        "history",
        history::run(
            history_identity,
            history_conflict,
            message_gaps,
            context,
            shutdown.clone()
        )
    );
    spawn!(
        "interaction-normal",
        interaction::run_normal(
            normal_interactions,
            interaction_handler.clone(),
            shutdown.clone()
        )
    );
    spawn!(
        "interaction-reserved",
        interaction::run_reserved(reserved_interactions, interaction_handler, shutdown.clone())
    );
    spawn!(
        "receive-error",
        receive_error::run(receive_errors, shutdown)
    );
    (handles, readiness)
}
