use cdr_core::{Validate, ValidationError, ValidationResult, count, length};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::identifiers::{
    TerminalId, TerminalReceiptId, TerminalWindowId, TerminalWindowObservationId,
    TerminalWindowReceiptId, validate_sha256,
};
use crate::request::{TerminalShell, TerminalWindowShell};

pub(crate) const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCwdScope {
    ProjectRoot,
    ProjectRelative,
    ExternalAbsolute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalExecutionReceipt {
    pub receipt_id: TerminalReceiptId,
    pub terminal_id: TerminalId,
    pub command_digest: String,
    pub shell: TerminalShell,
    pub cwd_scope: TerminalCwdScope,
    #[serde(default)]
    pub exit_code: Option<i64>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    pub truncated: bool,
}

impl Validate for TerminalExecutionReceipt {
    fn validate(&self) -> ValidationResult {
        self.receipt_id.validate()?;
        self.terminal_id.validate()?;
        validate_sha256("command_digest", &self.command_digest)?;
        if self.timed_out && self.cancelled {
            return Err(ValidationError::new(
                "outcome",
                "terminal execution cannot be timed out and cancelled",
            ));
        }
        if self.exit_code.is_none() && !(self.timed_out || self.cancelled) {
            return Err(ValidationError::new(
                "exit_code",
                "terminal execution without an exit code needs a stop reason",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalWindowEntry {
    pub terminal_window_id: TerminalWindowId,
    pub window_id: u64,
    pub process_id: u64,
    pub shell: TerminalWindowShell,
    pub cwd: String,
    pub title: String,
    #[serde(default = "default_true")]
    pub running: bool,
}

impl Validate for TerminalWindowEntry {
    fn validate(&self) -> ValidationResult {
        self.terminal_window_id.validate()?;
        if self.window_id == 0 || self.process_id == 0 {
            return Err(ValidationError::new(
                "window",
                "window and process IDs must be positive",
            ));
        }
        length("cwd", &self.cwd, 1, 1_000)?;
        length("title", &self.title, 1, 500)?;
        if self.running {
            Ok(())
        } else {
            Err(ValidationError::new("running", "must be true"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalWindowRect {
    pub left: i64,
    pub top: i64,
    pub width: u64,
    pub height: u64,
}

impl Validate for TerminalWindowRect {
    fn validate(&self) -> ValidationResult {
        if self.width > 0 && self.height > 0 {
            Ok(())
        } else {
            Err(ValidationError::new("rect", "dimensions must be positive"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalWindowAction {
    Activate,
    Type,
    Keys,
    Interrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalWindowActionReceipt {
    pub receipt_id: TerminalWindowReceiptId,
    pub terminal_window_id: TerminalWindowId,
    #[serde(default)]
    pub observation_id: Option<TerminalWindowObservationId>,
    pub identity_digest: String,
    pub action: TerminalWindowAction,
    #[serde(default)]
    pub unicode_chars: u32,
    #[serde(default)]
    pub keys: Vec<String>,
    pub activated: bool,
    pub completed_at: DateTime<Utc>,
}

impl Validate for TerminalWindowActionReceipt {
    fn validate(&self) -> ValidationResult {
        self.receipt_id.validate()?;
        self.terminal_window_id.validate()?;
        if let Some(id) = &self.observation_id {
            id.validate()?;
        }
        validate_sha256("identity_digest", &self.identity_digest)?;
        if self.unicode_chars > 4_096 {
            return Err(ValidationError::new(
                "unicode_chars",
                "must not exceed 4096",
            ));
        }
        count("keys", self.keys.len(), 0, 4)?;
        for key in &self.keys {
            length("keys", key, 1, 20)?;
        }
        let valid = match self.action {
            TerminalWindowAction::Activate => {
                self.observation_id.is_none() && self.unicode_chars == 0 && self.keys.is_empty()
            }
            TerminalWindowAction::Type => {
                self.observation_id.is_some() && self.unicode_chars > 0 && self.keys.is_empty()
            }
            TerminalWindowAction::Keys => {
                self.observation_id.is_some() && self.unicode_chars == 0 && !self.keys.is_empty()
            }
            TerminalWindowAction::Interrupt => {
                self.observation_id.is_some()
                    && self.unicode_chars == 0
                    && self.keys == ["CTRL", "C"]
            }
        };
        if valid {
            Ok(())
        } else {
            Err(ValidationError::new(
                "action",
                "terminal window action receipt fields are inconsistent",
            ))
        }
    }
}
