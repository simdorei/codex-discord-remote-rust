use cdr_core::deadline::default_request_deadline;
use cdr_core::{Validate, ValidationError, ValidationResult, length};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::request::ProjectOperation;

fn default_deadline() -> DateTime<Utc> {
    default_request_deadline()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayCommand {
    ProjectInfo {
        request_id: String,
        thread_id: String,
        #[serde(default = "default_deadline")]
        deadline_at: DateTime<Utc>,
        #[serde(default)]
        computer_session_id: Option<String>,
    },
    ListFiles {
        request_id: String,
        thread_id: String,
        #[serde(default = "default_deadline")]
        deadline_at: DateTime<Utc>,
        #[serde(default)]
        computer_session_id: Option<String>,
        pattern: String,
        limit: u16,
    },
    ReadFile {
        request_id: String,
        thread_id: String,
        #[serde(default = "default_deadline")]
        deadline_at: DateTime<Utc>,
        #[serde(default)]
        computer_session_id: Option<String>,
        path: String,
        start_line: u64,
        max_lines: u16,
    },
    WriteFile {
        request_id: String,
        thread_id: String,
        #[serde(default = "default_deadline")]
        deadline_at: DateTime<Utc>,
        #[serde(default)]
        computer_session_id: Option<String>,
        path: String,
        content: String,
        #[serde(default)]
        expected_sha256: Option<String>,
    },
    ProjectOperation {
        request_id: String,
        thread_id: String,
        #[serde(default = "default_deadline")]
        deadline_at: DateTime<Utc>,
        #[serde(default)]
        computer_session_id: Option<String>,
        operation: ProjectOperation,
    },
    ProjectSession {
        request_id: String,
        thread_id: String,
        #[serde(default = "default_deadline")]
        deadline_at: DateTime<Utc>,
        computer_session_id: String,
        computer_session_generation: u64,
    },
    DeviceSession {
        request_id: String,
        thread_id: String,
        #[serde(default = "default_deadline")]
        deadline_at: DateTime<Utc>,
        computer_session_id: String,
        computer_session_generation: u64,
        working_directory: String,
        expires_at: DateTime<Utc>,
    },
}

impl Validate for GatewayCommand {
    fn validate(&self) -> ValidationResult {
        if let Some(session) = self.computer_session_id() {
            length("computer_session_id", session, 16, 64)?;
        }
        match self {
            Self::ListFiles { pattern, limit, .. } => {
                length("pattern", pattern, 1, 500)?;
                within("limit", u64::from(*limit), 1, 500)
            }
            Self::ReadFile {
                path,
                start_line,
                max_lines,
                ..
            } => {
                length("path", path, 1, 1_000)?;
                within("start_line", *start_line, 1, u64::MAX)?;
                within("max_lines", u64::from(*max_lines), 1, 500)
            }
            Self::WriteFile {
                path,
                content,
                expected_sha256,
                ..
            } => {
                length("path", path, 1, 1_000)?;
                length("content", content, 0, 1_048_576)?;
                if let Some(value) = expected_sha256 {
                    length("expected_sha256", value, 64, 64)?;
                }
                Ok(())
            }
            Self::ProjectOperation { operation, .. } => operation.validate(),
            Self::ProjectSession {
                computer_session_generation,
                ..
            } => within(
                "computer_session_generation",
                *computer_session_generation,
                1,
                u64::MAX,
            ),
            Self::DeviceSession {
                computer_session_generation,
                working_directory,
                ..
            } => {
                within(
                    "computer_session_generation",
                    *computer_session_generation,
                    1,
                    u64::MAX,
                )?;
                length("working_directory", working_directory, 1, 1_000)
            }
            Self::ProjectInfo { .. } => Ok(()),
        }
    }
}

impl GatewayCommand {
    #[must_use]
    pub fn request_id(&self) -> &str {
        match self {
            Self::ProjectInfo { request_id, .. }
            | Self::ListFiles { request_id, .. }
            | Self::ReadFile { request_id, .. }
            | Self::WriteFile { request_id, .. }
            | Self::ProjectOperation { request_id, .. }
            | Self::ProjectSession { request_id, .. }
            | Self::DeviceSession { request_id, .. } => request_id,
        }
    }

    fn computer_session_id(&self) -> Option<&str> {
        match self {
            Self::ProjectInfo {
                computer_session_id,
                ..
            }
            | Self::ListFiles {
                computer_session_id,
                ..
            }
            | Self::ReadFile {
                computer_session_id,
                ..
            }
            | Self::WriteFile {
                computer_session_id,
                ..
            }
            | Self::ProjectOperation {
                computer_session_id,
                ..
            } => computer_session_id.as_deref(),
            Self::ProjectSession {
                computer_session_id,
                ..
            }
            | Self::DeviceSession {
                computer_session_id,
                ..
            } => Some(computer_session_id),
        }
    }
}

fn within(field: &'static str, value: u64, min: u64, max: u64) -> ValidationResult {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(ValidationError::new(
            field,
            format!("must be between {min} and {max}"),
        ))
    }
}
