//! Pure parsing of legacy `!command` input into runtime actions.

mod grammar;
mod parser;
mod settings;

use thiserror::Error;

pub use parser::plan_prefix;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MirrorDetailMode {
    Send,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillPromptKind {
    Pro,
    Interview,
    ArchiveUsed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrefixAction {
    Help,
    List {
        limit: u32,
    },
    ArchivedList {
        limit: u32,
    },
    Use {
        reference: String,
    },
    Open {
        reference: String,
        abort: bool,
    },
    Status {
        reference: Option<String>,
    },
    Stop {
        reference: Option<String>,
    },
    Settings {
        reference: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        speed: Option<String>,
    },
    AutoReserve {
        reference: Option<String>,
        enabled: bool,
    },
    SettingsOptions {
        reference: Option<String>,
        field: Option<String>,
    },
    DiscoverCodex,
    RestartCodex,
    ForceRestartCodex,
    Archive {
        reference: Option<String>,
    },
    DeleteArchivePreview {
        reference: String,
    },
    DeleteArchiveConfirm {
        reference: String,
    },
    Doctor,
    Resume {
        reference: Option<String>,
    },
    Identity,
    Where,
    Context {
        all_threads: bool,
        refresh: bool,
        limit: u32,
    },
    Usage {
        days: u32,
    },
    Runners,
    SavedRequest {
        request_id: String,
    },
    Resources,
    Retract {
        reference: Option<String>,
    },
    BridgeSync {
        limit: Option<u32>,
    },
    MirrorSync,
    MirrorList {
        limit: Option<u32>,
    },
    MirrorCheck {
        limit: Option<u32>,
    },
    MirrorDetail {
        mode: Option<MirrorDetailMode>,
    },
    Approval,
    New {
        prompt: String,
    },
    Steer {
        prompt: String,
    },
    QaButtons,
    SkillPrompt {
        kind: SkillPromptKind,
        request: String,
    },
    HostReboot,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PrefixPlanError {
    #[error("{0}")]
    Usage(String),
    #[error("unknown prefix command: !{0}")]
    Unknown(String),
}
