use std::sync::Arc;

use cdr_core::deadline::RequestBudget;
use cdr_remote_protocol::message::BridgeResult;
use cdr_remote_protocol::output::{CoreOutput, ProjectOperationOutput};
use cdr_remote_protocol::request::{CoreRequest, ProjectOperation, TerminalRequest};
use tokio::sync::watch;

use crate::commands::discover_commands;
use crate::{checkpoints, code, git, images};

use super::super::error::{
    checkpoint_error, code_error, command_error, computer_error, git_error, image_error,
    terminal_error,
};
use super::super::state::{ActiveProject, SessionActivation};

pub async fn computer_operation(
    request_id: &str,
    operation: &ProjectOperation,
    session: &Arc<SessionActivation>,
) -> Option<BridgeResult> {
    let ProjectOperation::Computer(request) = operation else {
        return None;
    };
    let request = request.clone();
    let session = Arc::clone(session);
    let output = match tokio::task::spawn_blocking(move || session.computer.execute(&request)).await
    {
        Ok(result) => result,
        Err(error) => Err(crate::computer::ComputerError::Platform(format!(
            "computer worker failed: {error}"
        ))),
    };
    Some(match output {
        Ok(output) => BridgeResult::ProjectOperationResult {
            request_id: request_id.into(),
            output: ProjectOperationOutput::Computer(output),
        },
        Err(error) => computer_error(request_id, &error),
    })
}

pub async fn terminal_operation(
    request_id: &str,
    operation: &ProjectOperation,
    session: &Arc<SessionActivation>,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Option<BridgeResult> {
    let ProjectOperation::Terminal(request) = operation else {
        return None;
    };
    let output = if matches!(request, TerminalRequest::TerminalExec { .. }) {
        session.terminals.execute(request, cancelled, budget).await
    } else {
        let request = request.clone();
        let session = Arc::clone(session);
        match tokio::task::spawn_blocking(move || session.terminal_windows.execute(&request)).await
        {
            Ok(result) => result,
            Err(error) => Err(crate::terminal::TerminalError::Window(format!(
                "terminal window worker failed: {error}"
            ))),
        }
    };
    Some(match output {
        Ok(output) => BridgeResult::ProjectOperationResult {
            request_id: request_id.into(),
            output: cdr_remote_protocol::output::ProjectOperationOutput::Terminal(output),
        },
        Err(error) => terminal_error(request_id, &error),
    })
}

pub async fn git_operation(
    request_id: &str,
    request: &CoreRequest,
    project: &ActiveProject,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Option<BridgeResult> {
    let output = match request {
        CoreRequest::RepoStatus => git::repo_status(&project.access, cancelled, budget).await,
        CoreRequest::RepoDiff => git::repo_diff(&project.access, cancelled, budget).await,
        CoreRequest::GitCommit { message, paths } => {
            git::commit(&project.access, message, paths, cancelled, budget).await
        }
        CoreRequest::GitPush { remote, branch } => {
            git::push(
                &project.access,
                remote,
                branch.as_deref(),
                cancelled,
                budget,
            )
            .await
        }
        _ => return None,
    };
    Some(match output {
        Ok(output) => result(request_id, output),
        Err(error) => git_error(request_id, &error),
    })
}

pub async fn image_operation(
    request_id: &str,
    request: &CoreRequest,
    project: &ActiveProject,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Option<BridgeResult> {
    let output = match request {
        CoreRequest::SaveImage {
            path,
            data_base64,
            overwrite,
        } => images::save(&project.access, path, data_base64, *overwrite),
        CoreRequest::SaveImageFromUrl {
            path,
            url,
            overwrite,
        } => images::save_from_url(&project.access, path, url, *overwrite, cancelled, budget).await,
        CoreRequest::ListImages => images::list(&project.access, &cancelled),
        CoreRequest::RetrieveImage { path } => images::retrieve(&project.access, path, &cancelled),
        _ => return None,
    };
    Some(match output {
        Ok(output) => result(request_id, output),
        Err(error) => image_error(request_id, &error),
    })
}

pub fn checkpoint_operation(
    request_id: &str,
    request: &CoreRequest,
    project: &ActiveProject,
) -> Option<BridgeResult> {
    let output = match request {
        CoreRequest::CheckpointList => checkpoints::list(project.access.root())
            .map(|checkpoints| CoreOutput::CheckpointList { checkpoints }),
        CoreRequest::CheckpointShow { checkpoint_id } => {
            checkpoints::show(project.access.root(), checkpoint_id)
                .map(|(checkpoint, patch)| CoreOutput::CheckpointShow { checkpoint, patch })
        }
        CoreRequest::CheckpointRestore { checkpoint_id } => {
            checkpoints::restore(&project.access, checkpoint_id).map(|restored_files| {
                CoreOutput::CheckpointRestore {
                    checkpoint_id: checkpoint_id.clone(),
                    restored_files,
                }
            })
        }
        _ => return None,
    };
    Some(match output {
        Ok(output) => result(request_id, output),
        Err(error) => checkpoint_error(request_id, &error),
    })
}

pub async fn project_status(
    request_id: &str,
    project: &ActiveProject,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> BridgeResult {
    let status = match git::repo_status(&project.access, cancelled.clone(), budget).await {
        Ok(CoreOutput::RepoStatus {
            branch,
            dirty_files,
            staged_files,
            ..
        }) => (branch, dirty_files, staged_files),
        Ok(_) => unreachable!(),
        Err(error) => return git_error(request_id, &error),
    };
    let rules = match code::project_rules(&project.access, &cancelled) {
        Ok(CoreOutput::ProjectRules { rules }) => rules,
        Ok(_) => unreachable!(),
        Err(error) => return code_error(request_id, &error),
    };
    let commands = match discover_commands(&project.access) {
        Ok(commands) => commands,
        Err(error) => return command_error(request_id, &error),
    };
    result(
        request_id,
        CoreOutput::ProjectStatus {
            branch: status.0,
            dirty_files: status.1,
            staged_files: status.2,
            rule_files: rules.into_iter().map(|rule| rule.path).collect(),
            command_ids: commands
                .into_iter()
                .map(|command| command.descriptor.command_id)
                .collect(),
        },
    )
}

fn result(request_id: &str, output: CoreOutput) -> BridgeResult {
    BridgeResult::ProjectOperationResult {
        request_id: request_id.into(),
        output: ProjectOperationOutput::Core(output),
    }
}
