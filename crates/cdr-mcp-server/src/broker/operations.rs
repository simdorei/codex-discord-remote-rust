use cdr_core::Validate;
use cdr_remote_protocol::message::{
    BridgeResult, GatewayCommand, ListFilesOutput, ReadFileOutput, WriteFileOutput,
};
use cdr_remote_protocol::{ProjectOperation, ProjectOperationOutput};
use tokio_util::sync::CancellationToken;

use super::BridgeBroker;
use super::dispatch::request_id;
use super::error::BrokerError;

impl BridgeBroker {
    pub async fn list_files(
        &self,
        session: &str,
        subject: &str,
        pattern: String,
        limit: u16,
        cancellation: CancellationToken,
    ) -> Result<ListFilesOutput, BrokerError> {
        let route = self.active_route(session, subject).await?;
        let command = GatewayCommand::ListFiles {
            request_id: request_id(),
            thread_id: route.thread_id.clone(),
            deadline_at: cdr_core::deadline::default_request_deadline(),
            computer_session_id: Some(route.computer_session_id.clone()),
            pattern,
            limit,
        };
        match self
            .dispatch_on_route_cancellable(&route, command, cancellation)
            .await?
        {
            BridgeResult::ListFilesResult { output, .. } => Ok(output),
            BridgeResult::OperationError { message, .. } => remote_error(message),
            _ => Err(BrokerError::WrongResultType),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn read_file(
        &self,
        session: &str,
        subject: &str,
        call_request_id: String,
        path: String,
        start_line: u64,
        max_lines: u16,
        cancellation: CancellationToken,
    ) -> Result<ReadFileOutput, BrokerError> {
        let route = self.active_route(session, subject).await?;
        let command = GatewayCommand::ReadFile {
            request_id: call_request_id,
            thread_id: route.thread_id.clone(),
            deadline_at: cdr_core::deadline::default_request_deadline(),
            computer_session_id: Some(route.computer_session_id.clone()),
            path,
            start_line,
            max_lines,
        };
        match self
            .dispatch_on_route_cancellable(&route, command, cancellation)
            .await?
        {
            BridgeResult::ReadFileResult { output, .. } => Ok(output),
            BridgeResult::OperationError { message, .. } => remote_error(message),
            _ => Err(BrokerError::WrongResultType),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn write_file(
        &self,
        session: &str,
        subject: &str,
        call_request_id: String,
        path: String,
        content: String,
        expected_sha256: Option<String>,
        cancellation: CancellationToken,
    ) -> Result<WriteFileOutput, BrokerError> {
        let route = self.active_route(session, subject).await?;
        let command = GatewayCommand::WriteFile {
            request_id: call_request_id,
            thread_id: route.thread_id.clone(),
            deadline_at: cdr_core::deadline::default_request_deadline(),
            computer_session_id: Some(route.computer_session_id.clone()),
            path,
            content,
            expected_sha256,
        };
        match self
            .dispatch_on_route_cancellable(&route, command, cancellation)
            .await?
        {
            BridgeResult::WriteFileResult { output, .. } => Ok(output),
            BridgeResult::OperationError { message, .. } => remote_error(message),
            _ => Err(BrokerError::WrongResultType),
        }
    }

    pub async fn project_operation(
        &self,
        session: &str,
        subject: &str,
        call_request_id: String,
        operation: ProjectOperation,
        cancellation: CancellationToken,
    ) -> Result<ProjectOperationOutput, BrokerError> {
        operation
            .validate()
            .map_err(|error| BrokerError::InvalidRequest(error.to_string()))?;
        let route = self.active_route(session, subject).await?;
        let deadline_at = operation.request_deadline();
        let command = GatewayCommand::ProjectOperation {
            request_id: call_request_id,
            thread_id: route.thread_id.clone(),
            deadline_at,
            computer_session_id: Some(route.computer_session_id.clone()),
            operation,
        };
        match self
            .dispatch_on_route_cancellable(&route, command, cancellation)
            .await?
        {
            BridgeResult::ProjectOperationResult { output, .. } => Ok(output),
            BridgeResult::OperationError { message, .. } => remote_error(message),
            _ => Err(BrokerError::WrongResultType),
        }
    }
}

fn remote_error<T>(message: String) -> Result<T, BrokerError> {
    Err(BrokerError::RemoteOperation(message))
}
