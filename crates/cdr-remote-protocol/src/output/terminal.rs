use cdr_core::{Validate, ValidationError, ValidationResult, length};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::terminal_types::{TerminalExecutionReceipt, TerminalWindowActionReceipt, default_true};
use super::{TerminalWindowEntry, TerminalWindowRect};
use crate::identifiers::{
    TerminalId, TerminalWindowId, TerminalWindowObservationId, validate_sha256,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalCaptureMediaType {
    #[default]
    #[serde(rename = "image/png")]
    Png,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalOutput {
    TerminalExec {
        terminal_id: TerminalId,
        process_id: u64,
        #[serde(default)]
        exit_code: Option<i64>,
        stdout: String,
        stderr: String,
        cwd: String,
        duration_ms: u64,
        timed_out: bool,
        cancelled: bool,
        truncated: bool,
        receipt: TerminalExecutionReceipt,
    },
    TerminalWindowOpen {
        window: TerminalWindowEntry,
    },
    TerminalWindowList {
        windows: Vec<TerminalWindowEntry>,
    },
    TerminalWindowClose {
        terminal_window_id: TerminalWindowId,
        #[serde(default = "default_true")]
        closed: bool,
    },
    TerminalWindowCapture {
        window: TerminalWindowEntry,
        observation_id: TerminalWindowObservationId,
        identity_digest: String,
        rect: TerminalWindowRect,
        #[serde(default)]
        media_type: TerminalCaptureMediaType,
        data_base64: String,
        captured_at: DateTime<Utc>,
    },
    TerminalWindowAction {
        window: TerminalWindowEntry,
        receipt: TerminalWindowActionReceipt,
    },
}

impl Validate for TerminalOutput {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::TerminalExec {
                terminal_id,
                process_id,
                exit_code,
                stdout,
                stderr,
                cwd,
                duration_ms,
                timed_out,
                cancelled,
                truncated,
                receipt,
            } => {
                terminal_id.validate()?;
                if *process_id == 0 {
                    return Err(ValidationError::new("process_id", "must be positive"));
                }
                length("stdout", stdout, 0, 1_048_576)?;
                length("stderr", stderr, 0, 1_048_576)?;
                length("cwd", cwd, 1, 1_000)?;
                receipt.validate()?;
                if receipt.terminal_id != *terminal_id
                    || receipt.exit_code != *exit_code
                    || receipt.duration_ms != *duration_ms
                    || receipt.timed_out != *timed_out
                    || receipt.cancelled != *cancelled
                    || receipt.truncated != *truncated
                {
                    return Err(ValidationError::new(
                        "receipt",
                        "terminal output and receipt fields must match",
                    ));
                }
                Ok(())
            }
            Self::TerminalWindowOpen { window } => window.validate(),
            Self::TerminalWindowList { windows } => windows.iter().try_for_each(Validate::validate),
            Self::TerminalWindowClose {
                terminal_window_id,
                closed,
            } => {
                terminal_window_id.validate()?;
                if *closed {
                    Ok(())
                } else {
                    Err(ValidationError::new("closed", "must be true"))
                }
            }
            Self::TerminalWindowCapture {
                window,
                observation_id,
                identity_digest,
                rect,
                data_base64,
                ..
            } => {
                window.validate()?;
                observation_id.validate()?;
                validate_sha256("identity_digest", identity_digest)?;
                rect.validate()?;
                length("data_base64", data_base64, 12, 12_000_000)
            }
            Self::TerminalWindowAction { window, receipt } => {
                window.validate()?;
                receipt.validate()?;
                if window.terminal_window_id == receipt.terminal_window_id {
                    Ok(())
                } else {
                    Err(ValidationError::new(
                        "receipt",
                        "terminal window receipt identity does not match output",
                    ))
                }
            }
        }
    }
}
