use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use chrono::{DateTime, Utc};

use crate::checkpoints::CheckpointError;
use crate::code::CodeError;
use crate::commands::CommandError;
use crate::computer::ComputerError;
use crate::files::RemoteFileError;
use crate::files::redaction::redact;
use crate::git::GitError;
use crate::images::ImageError;
use crate::terminal::TerminalError;

pub struct CommandMeta<'a> {
    pub request_id: &'a str,
    pub thread_id: &'a str,
    pub deadline_at: DateTime<Utc>,
    pub computer_session_id: Option<&'a str>,
}

pub fn command_meta(command: &GatewayCommand) -> CommandMeta<'_> {
    macro_rules! meta {
        ($request_id:expr, $thread_id:expr, $deadline_at:expr, $session:expr) => {
            CommandMeta {
                request_id: $request_id,
                thread_id: $thread_id,
                deadline_at: *$deadline_at,
                computer_session_id: $session,
            }
        };
    }
    match command {
        GatewayCommand::ProjectInfo {
            request_id,
            thread_id,
            deadline_at,
            computer_session_id,
        }
        | GatewayCommand::ListFiles {
            request_id,
            thread_id,
            deadline_at,
            computer_session_id,
            ..
        }
        | GatewayCommand::ReadFile {
            request_id,
            thread_id,
            deadline_at,
            computer_session_id,
            ..
        }
        | GatewayCommand::WriteFile {
            request_id,
            thread_id,
            deadline_at,
            computer_session_id,
            ..
        }
        | GatewayCommand::ProjectOperation {
            request_id,
            thread_id,
            deadline_at,
            computer_session_id,
            ..
        } => meta!(
            request_id,
            thread_id,
            deadline_at,
            computer_session_id.as_deref()
        ),
        GatewayCommand::ProjectSession {
            request_id,
            thread_id,
            deadline_at,
            computer_session_id,
            ..
        }
        | GatewayCommand::DeviceSession {
            request_id,
            thread_id,
            deadline_at,
            computer_session_id,
            ..
        } => meta!(
            request_id,
            thread_id,
            deadline_at,
            Some(computer_session_id)
        ),
    }
}

pub fn binding_error(request_id: &str, code: &str) -> BridgeResult {
    let message = match code {
        "binding_expired" => "The local project binding expired. Run !pro again.",
        _ => "The Codex thread is not bound on this device.",
    };
    operation_error(request_id, code, message)
}

pub fn expired_result(request_id: &str) -> BridgeResult {
    operation_error(
        request_id,
        "request_expired",
        "The remote project request expired before it could run.",
    )
}

pub fn cancelled_result(request_id: &str) -> BridgeResult {
    operation_error(
        request_id,
        "request_cancelled",
        "The remote project request was cancelled because the local bridge disconnected.",
    )
}

pub fn session_error(request_id: &str, message: &str) -> BridgeResult {
    operation_error(request_id, "computer_control", message)
}

pub fn file_error(request_id: &str, error: &RemoteFileError) -> BridgeResult {
    let code = match error {
        RemoteFileError::UnsafePath { .. } | RemoteFileError::UnsafePattern { .. } => {
            "unsafeprojectpath"
        }
        RemoteFileError::Size { .. } => "projectfilesize",
        RemoteFileError::Encoding { .. } => "projectfileencoding",
        RemoteFileError::Conflict { .. } => "fileconflict",
        RemoteFileError::Limit { .. } => "projectfilelimit",
        RemoteFileError::Io(_) => "projectfile",
    };
    operation_error(request_id, code, &redact(&error.to_string()))
}

pub fn command_error(request_id: &str, error: &CommandError) -> BridgeResult {
    operation_error(request_id, "projectcommand", &redact(&error.to_string()))
}

pub fn checkpoint_error(request_id: &str, error: &CheckpointError) -> BridgeResult {
    if let CheckpointError::File(error) = error {
        return file_error(request_id, error);
    }
    operation_error(request_id, "projectcheckpoint", &redact(&error.to_string()))
}

pub fn code_error(request_id: &str, error: &CodeError) -> BridgeResult {
    match error {
        CodeError::File(error) => file_error(request_id, error),
        CodeError::Cancelled => cancelled_result(request_id),
    }
}

pub fn git_error(request_id: &str, error: &GitError) -> BridgeResult {
    if let GitError::File(error) = error {
        return file_error(request_id, error);
    }
    operation_error(request_id, "projectgit", &redact(&error.to_string()))
}

pub fn image_error(request_id: &str, error: &ImageError) -> BridgeResult {
    match error {
        ImageError::File(error) => file_error(request_id, error),
        ImageError::Checkpoint(error) => checkpoint_error(request_id, error),
        ImageError::Cancelled => cancelled_result(request_id),
        _ => operation_error(request_id, "projectimage", &redact(&error.to_string())),
    }
}

pub fn terminal_error(request_id: &str, error: &TerminalError) -> BridgeResult {
    operation_error(
        request_id,
        "terminal_execution",
        &redact(&error.to_string()),
    )
}

pub fn computer_error(request_id: &str, error: &ComputerError) -> BridgeResult {
    let code = if matches!(error, ComputerError::Stopped) {
        "computer_control_stopped"
    } else {
        "computer_control"
    };
    operation_error(request_id, code, &redact(&error.to_string()))
}

pub fn unsupported(request_id: &str) -> BridgeResult {
    operation_error(
        request_id,
        "projectcapability",
        "This project capability is not implemented by the Rust local agent yet.",
    )
}

fn operation_error(request_id: &str, code: &str, message: &str) -> BridgeResult {
    BridgeResult::OperationError {
        request_id: request_id.to_owned(),
        error_code: code.to_owned(),
        message: message.to_owned(),
    }
}
