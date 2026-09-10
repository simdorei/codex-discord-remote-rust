use std::collections::BTreeMap;

use cdr_core::{Validate, ValidationError, ValidationResult, count, length};
use serde::{Deserialize, Serialize};

use crate::identifiers::{TerminalId, TerminalWindowId, TerminalWindowObservationId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalShell {
    Auto,
    Powershell,
    Cmd,
    Sh,
    Bash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalWindowShell {
    Powershell,
    Cmd,
}

fn default_terminal_shell() -> TerminalShell {
    TerminalShell::Auto
}
fn default_terminal_timeout() -> u16 {
    300
}
fn default_window_shell() -> TerminalWindowShell {
    TerminalWindowShell::Powershell
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalRequest {
    TerminalExec {
        #[serde(default)]
        terminal_id: Option<TerminalId>,
        #[serde(default = "default_terminal_shell")]
        shell: TerminalShell,
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        #[serde(default = "default_terminal_timeout")]
        timeout_seconds: u16,
        #[serde(default)]
        cancel_previous: bool,
    },
    TerminalWindowOpen {
        #[serde(default = "default_window_shell")]
        shell: TerminalWindowShell,
        #[serde(default)]
        cwd: Option<String>,
    },
    TerminalWindowList,
    TerminalWindowClose {
        terminal_window_id: TerminalWindowId,
    },
    TerminalWindowCapture {
        terminal_window_id: TerminalWindowId,
    },
    TerminalWindowActivate {
        terminal_window_id: TerminalWindowId,
    },
    TerminalWindowType {
        terminal_window_id: TerminalWindowId,
        observation_id: TerminalWindowObservationId,
        text: String,
    },
    TerminalWindowKeys {
        terminal_window_id: TerminalWindowId,
        observation_id: TerminalWindowObservationId,
        keys: Vec<String>,
    },
    TerminalWindowInterrupt {
        terminal_window_id: TerminalWindowId,
        observation_id: TerminalWindowObservationId,
    },
}

impl Validate for TerminalRequest {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::TerminalExec {
                terminal_id,
                command,
                cwd,
                environment,
                timeout_seconds,
                ..
            } => {
                if let Some(id) = terminal_id {
                    id.validate()?;
                }
                length("command", command, 1, 32_768)?;
                if let Some(cwd) = cwd {
                    length("cwd", cwd, 1, 1_000)?;
                }
                count("environment", environment.len(), 0, 100)?;
                for (name, value) in environment {
                    length("environment.name", name, 1, 1_024)?;
                    if name.contains(['=', '\0']) {
                        return Err(ValidationError::new(
                            "environment.name",
                            "must not contain '=' or NUL",
                        ));
                    }
                    length("environment.value", value, 0, 32_767)?;
                    if value.contains('\0') {
                        return Err(ValidationError::new(
                            "environment.value",
                            "must not contain NUL",
                        ));
                    }
                }
                if !(1..=3_600).contains(timeout_seconds) {
                    return Err(ValidationError::new(
                        "timeout_seconds",
                        "must be between 1 and 3600",
                    ));
                }
                Ok(())
            }
            Self::TerminalWindowOpen { cwd, .. } => {
                if let Some(cwd) = cwd {
                    length("cwd", cwd, 1, 1_000)?;
                }
                Ok(())
            }
            Self::TerminalWindowList => Ok(()),
            Self::TerminalWindowClose { terminal_window_id }
            | Self::TerminalWindowCapture { terminal_window_id }
            | Self::TerminalWindowActivate { terminal_window_id } => terminal_window_id.validate(),
            Self::TerminalWindowType {
                terminal_window_id,
                observation_id,
                text,
            } => {
                terminal_window_id.validate()?;
                observation_id.validate()?;
                length("text", text, 1, 4_096)
            }
            Self::TerminalWindowKeys {
                terminal_window_id,
                observation_id,
                keys,
            } => {
                terminal_window_id.validate()?;
                observation_id.validate()?;
                count("keys", keys.len(), 1, 4)?;
                for key in keys {
                    length("keys", key, 1, 20)?;
                }
                Ok(())
            }
            Self::TerminalWindowInterrupt {
                terminal_window_id,
                observation_id,
            } => {
                terminal_window_id.validate()?;
                observation_id.validate()
            }
        }
    }
}
