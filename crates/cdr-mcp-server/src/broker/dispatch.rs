use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::BridgeBroker;
use super::error::BrokerError;
use super::model::SessionRoute;
use super::state::PendingRequest;

impl BridgeBroker {
    pub(super) async fn dispatch_on_route(
        &self,
        route: &SessionRoute,
        command: GatewayCommand,
    ) -> Result<BridgeResult, BrokerError> {
        self.dispatch_on_route_cancellable(route, command, CancellationToken::new())
            .await
    }

    pub(super) async fn dispatch_on_route_cancellable(
        &self,
        route: &SessionRoute,
        command: GatewayCommand,
        cancellation: CancellationToken,
    ) -> Result<BridgeResult, BrokerError> {
        let request_id = command.request_id().to_owned();
        let (response, response_rx) = oneshot::channel();
        let commands = {
            let mut state = self.state.lock().await;
            let current = state
                .sessions
                .get(&route.session)
                .filter(|value| value.computer_session_id == route.computer_session_id)
                .ok_or(BrokerError::ActiveSelectionMissing)?;
            let (commands, connection_id) = state
                .devices
                .get(&current.device_id)
                .map(|connection| (connection.commands.clone(), connection.connection_id))
                .ok_or(BrokerError::BridgeUnavailable)?;
            state.pending.insert(
                request_id.clone(),
                PendingRequest {
                    device_id: route.device_id.clone(),
                    connection_id,
                    response,
                },
            );
            commands
        };
        let sent = tokio::select! {
            result = tokio::time::timeout(self.request_timeout, commands.send(command)) => result,
            () = cancellation.cancelled() => {
                self.state.lock().await.pending.remove(&request_id);
                return Err(BrokerError::Cancelled);
            }
        };
        if !matches!(sent, Ok(Ok(()))) {
            self.state.lock().await.pending.remove(&request_id);
            return Err(BrokerError::BridgeUnavailable);
        }
        let response_result = tokio::select! {
            result = tokio::time::timeout(self.request_timeout, response_rx) => result,
            () = cancellation.cancelled() => {
                self.state.lock().await.pending.remove(&request_id);
                return Err(BrokerError::Cancelled);
            }
        };
        match response_result {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => Err(BrokerError::MissingResult),
            Err(_) => {
                self.state.lock().await.pending.remove(&request_id);
                Err(BrokerError::ResponseTimeout)
            }
        }
    }

    pub(super) async fn remove_route_if_current(&self, route: &SessionRoute) {
        let mut state = self.state.lock().await;
        if state
            .sessions
            .get(&route.session)
            .is_some_and(|value| value.computer_session_id == route.computer_session_id)
        {
            state.sessions.remove(&route.session);
        }
    }
}

pub(super) fn require_session_result(result: BridgeResult) -> Result<(), BrokerError> {
    match result {
        BridgeResult::ProjectSessionResult { .. } => Ok(()),
        BridgeResult::OperationError { message, .. } => Err(BrokerError::RemoteOperation(message)),
        _ => Err(BrokerError::WrongResultType),
    }
}

pub(super) fn request_id() -> String {
    Uuid::new_v4().simple().to_string()
}
