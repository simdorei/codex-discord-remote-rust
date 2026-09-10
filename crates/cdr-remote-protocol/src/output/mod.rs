mod computer;
mod core;
mod terminal;
mod terminal_types;

pub use computer::{
    ComputerActionName, ComputerOutput, ComputerScreenshotMediaType, ComputerWindowEntry,
};
pub use core::{
    CheckpointEntry, CommandDescriptor, CoreOutput, DiffFile, ImageEntry, PatchAction, PatchEntry,
    RiskTier, RuleFile, SearchMatch,
};
pub use terminal::{TerminalCaptureMediaType, TerminalOutput};
pub use terminal_types::{
    TerminalCwdScope, TerminalExecutionReceipt, TerminalWindowAction, TerminalWindowActionReceipt,
    TerminalWindowEntry, TerminalWindowRect,
};

pub const ALL_PROJECT_OUTPUT_KINDS: &[&str] = &[
    "checkpoint_list",
    "checkpoint_restore",
    "checkpoint_show",
    "code_search",
    "command_list",
    "command_run",
    "computer_action",
    "computer_screenshot",
    "computer_stop",
    "computer_windows",
    "file_apply_patch",
    "file_create",
    "git_commit",
    "git_push",
    "image_list",
    "image_retrieve",
    "image_save",
    "project_rules",
    "project_status",
    "repo_diff",
    "repo_status",
    "terminal_exec",
    "terminal_window_action",
    "terminal_window_capture",
    "terminal_window_close",
    "terminal_window_list",
    "terminal_window_open",
];

use cdr_core::{Validate, ValidationResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProjectOperationOutput {
    Core(CoreOutput),
    Computer(ComputerOutput),
    Terminal(TerminalOutput),
}

impl Validate for ProjectOperationOutput {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::Core(value) => value.validate(),
            Self::Computer(value) => value.validate(),
            Self::Terminal(value) => value.validate(),
        }
    }
}
