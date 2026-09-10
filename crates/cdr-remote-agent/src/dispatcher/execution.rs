mod helpers;

use std::sync::Arc;
use std::time::Duration;

use cdr_core::deadline::RequestBudget;
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::output::{CoreOutput, ProjectOperationOutput};
use cdr_remote_protocol::request::{CoreRequest, ProjectOperation};
use chrono::Utc;

use tokio::sync::watch;

use crate::commands::{discover_commands, run_command_cancellable};
use crate::{code, operations, patches};

use super::error::{
    checkpoint_error, code_error, command_error, command_meta, file_error, unsupported,
};
use super::state::{ActiveProject, SessionActivation};
use helpers::{
    checkpoint_operation, computer_operation, git_operation, image_operation, project_status,
    terminal_operation,
};

pub async fn execute(
    command: &GatewayCommand,
    project: &ActiveProject,
    session: &Arc<SessionActivation>,
    cancelled: watch::Receiver<bool>,
) -> BridgeResult {
    let request_id = command_meta(command).request_id;
    match command {
        GatewayCommand::ProjectInfo { thread_id, .. } => BridgeResult::ProjectInfoResult {
            request_id: request_id.into(),
            output: project.access.project_info(thread_id),
        },
        GatewayCommand::ListFiles { pattern, limit, .. } => {
            match project.access.list_files(pattern, *limit) {
                Ok(output) => BridgeResult::ListFilesResult {
                    request_id: request_id.into(),
                    output,
                },
                Err(error) => file_error(request_id, &error),
            }
        }
        GatewayCommand::ReadFile {
            path,
            start_line,
            max_lines,
            ..
        } => match project.access.read_file(path, *start_line, *max_lines) {
            Ok(output) => BridgeResult::ReadFileResult {
                request_id: request_id.into(),
                output,
            },
            Err(error) => file_error(request_id, &error),
        },
        GatewayCommand::WriteFile {
            path,
            content,
            expected_sha256,
            ..
        } => match operations::write_with_checkpoint(
            &project.access,
            path,
            content,
            expected_sha256.as_deref(),
        ) {
            Ok(output) => BridgeResult::WriteFileResult {
                request_id: request_id.into(),
                output,
            },
            Err(error) => checkpoint_error(request_id, &error),
        },
        GatewayCommand::ProjectOperation {
            operation,
            deadline_at,
            ..
        } => {
            execute_operation(
                request_id,
                operation,
                *deadline_at,
                project,
                session,
                cancelled,
            )
            .await
        }
        GatewayCommand::ProjectSession { .. } | GatewayCommand::DeviceSession { .. } => {
            unsupported(request_id)
        }
    }
}

async fn execute_operation(
    request_id: &str,
    operation: &ProjectOperation,
    deadline_at: chrono::DateTime<Utc>,
    project: &ActiveProject,
    session: &Arc<SessionActivation>,
    cancelled: watch::Receiver<bool>,
) -> BridgeResult {
    let budget = RequestBudget::from_deadline(deadline_at);
    if let Some(result) = computer_operation(request_id, operation, session).await {
        return result;
    }
    if let Some(result) =
        terminal_operation(request_id, operation, session, cancelled.clone(), &budget).await
    {
        return result;
    }
    if let ProjectOperation::Core(core) = operation
        && let Some(result) = checkpoint_operation(request_id, core, project)
    {
        return result;
    }
    if let ProjectOperation::Core(core) = operation
        && let Some(result) =
            git_operation(request_id, core, project, cancelled.clone(), &budget).await
    {
        return result;
    }
    if let ProjectOperation::Core(core) = operation
        && let Some(result) =
            image_operation(request_id, core, project, cancelled.clone(), &budget).await
    {
        return result;
    }
    let output = match operation {
        ProjectOperation::Core(CoreRequest::ProjectRules) => {
            match code::project_rules(&project.access, &cancelled) {
                Ok(output) => output,
                Err(error) => return code_error(request_id, &error),
            }
        }
        ProjectOperation::Core(CoreRequest::CodeSearch { query, max_results }) => {
            match code::search(&project.access, query, *max_results, &cancelled) {
                Ok(output) => output,
                Err(error) => return code_error(request_id, &error),
            }
        }
        ProjectOperation::Core(CoreRequest::ProjectStatus) => {
            return project_status(request_id, project, cancelled, &budget).await;
        }
        ProjectOperation::Core(CoreRequest::FileCreate {
            path,
            content,
            overwrite,
        }) => match operations::create_file(&project.access, path, content, *overwrite) {
            Ok(output) => output,
            Err(error) => return checkpoint_error(request_id, &error),
        },
        ProjectOperation::Core(CoreRequest::FileApplyPatch { changes }) => {
            match patches::apply(&project.access, changes) {
                Ok(output) => output,
                Err(error) => return checkpoint_error(request_id, &error),
            }
        }
        ProjectOperation::Core(CoreRequest::CommandList) => {
            match discover_commands(&project.access) {
                Ok(commands) => CoreOutput::CommandList {
                    commands: commands
                        .into_iter()
                        .map(|command| command.descriptor)
                        .collect(),
                },
                Err(error) => return command_error(request_id, &error),
            }
        }
        ProjectOperation::Core(CoreRequest::CommandRun {
            command_id,
            timeout_seconds,
        }) => {
            let remaining = (deadline_at - Utc::now())
                .num_seconds()
                .max(1)
                .min(i64::from(*timeout_seconds));
            match run_command_cancellable(
                &project.access,
                command_id,
                Duration::from_secs(u64::try_from(remaining).unwrap_or(1)),
                None,
                cancelled,
            )
            .await
            {
                Ok(output) => output,
                Err(error) => return command_error(request_id, &error),
            }
        }
        _ => return unsupported(request_id),
    };
    BridgeResult::ProjectOperationResult {
        request_id: request_id.into(),
        output: ProjectOperationOutput::Core(output),
    }
}
