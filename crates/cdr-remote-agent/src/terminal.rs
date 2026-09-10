mod engine;
mod shell;
mod window;

use cdr_core::deadline::DeadlineError;
use cdr_remote_protocol::request::TerminalShell;
use thiserror::Error;

use crate::commands::ProcessError;

pub use engine::TerminalExecutionEngine;
pub(crate) use shell::inherited_environment;
pub use window::{
    OwnedTerminalWindow, TerminalWindowBackend, TerminalWindowCapture,
    TerminalWindowInteractionBackend, TerminalWindowManager, TerminalWindowObservation,
};

pub const MAX_TERMINAL_STREAM_BYTES: usize = 1_048_576;

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("session_id must not be empty")]
    EmptySession,
    #[error("terminal root is not a directory")]
    InvalidRoot,
    #[error("terminal session is closed")]
    Closed,
    #[error("terminal does not belong to this session")]
    ForeignTerminal,
    #[error("terminal already has an active command")]
    ActiveCommand,
    #[error("terminal execution was cancelled")]
    Cancelled,
    #[error("requested terminal shell is unavailable: {0:?}")]
    ShellUnavailable(TerminalShell),
    #[error("terminal executable or directory was not found")]
    MissingExecutableOrDirectory,
    #[error("terminal process trees did not stop before close timed out")]
    CloseTimeout,
    #[error("terminal window operation is not an execution request")]
    UnsupportedRequest,
    #[error("{0}")]
    Window(String),
    #[error(transparent)]
    Deadline(#[from] DeadlineError),
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("terminal path could not be resolved: {0}")]
    Io(#[from] std::io::Error),
}
