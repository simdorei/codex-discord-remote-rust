mod computer;
mod core;
mod terminal;

pub use computer::{ComputerApp, ComputerMouseButton, ComputerRequest};
pub use core::CoreRequest;
pub use terminal::{TerminalRequest, TerminalShell, TerminalWindowShell};

pub const ALL_PROJECT_OPERATION_KINDS: &[&str] = &[
    "checkpoint_list",
    "checkpoint_restore",
    "checkpoint_show",
    "code_search",
    "command_list",
    "command_run",
    "computer_activate",
    "computer_click",
    "computer_close",
    "computer_drag",
    "computer_launch",
    "computer_list_windows",
    "computer_press_keys",
    "computer_screenshot",
    "computer_scroll",
    "computer_set_clipboard",
    "computer_stop",
    "computer_type_text",
    "file_apply_patch",
    "file_create",
    "git_commit",
    "git_push",
    "list_images",
    "project_rules",
    "project_status",
    "repo_diff",
    "repo_status",
    "retrieve_image",
    "save_image",
    "save_image_from_url",
    "terminal_exec",
    "terminal_window_activate",
    "terminal_window_capture",
    "terminal_window_close",
    "terminal_window_interrupt",
    "terminal_window_keys",
    "terminal_window_list",
    "terminal_window_open",
    "terminal_window_type",
];

use cdr_core::deadline::{
    DEFAULT_REQUEST_LIFETIME_SECONDS, GIT_REQUEST_LIFETIME_SECONDS, TRANSPORT_GRACE_SECONDS,
};
use cdr_core::{Validate, ValidationResult};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProjectOperation {
    Core(CoreRequest),
    Computer(ComputerRequest),
    Terminal(TerminalRequest),
}

impl Validate for ProjectOperation {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::Core(value) => value.validate(),
            Self::Computer(value) => value.validate(),
            Self::Terminal(value) => value.validate(),
        }
    }
}

impl ProjectOperation {
    #[must_use]
    pub fn request_lifetime_seconds(&self) -> i64 {
        match self {
            Self::Core(CoreRequest::CommandRun {
                timeout_seconds, ..
            })
            | Self::Terminal(TerminalRequest::TerminalExec {
                timeout_seconds, ..
            }) => i64::from(*timeout_seconds) + TRANSPORT_GRACE_SECONDS,
            Self::Core(CoreRequest::GitPush { .. }) => 300 + TRANSPORT_GRACE_SECONDS,
            Self::Core(
                CoreRequest::RepoStatus
                | CoreRequest::RepoDiff
                | CoreRequest::GitCommit { .. }
                | CoreRequest::ProjectStatus,
            ) => GIT_REQUEST_LIFETIME_SECONDS,
            _ => DEFAULT_REQUEST_LIFETIME_SECONDS,
        }
    }

    #[must_use]
    pub fn request_deadline(&self) -> DateTime<Utc> {
        Utc::now() + TimeDelta::seconds(self.request_lifetime_seconds())
    }
}
