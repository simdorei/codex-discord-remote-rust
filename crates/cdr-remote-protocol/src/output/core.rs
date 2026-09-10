use cdr_core::{Validate, ValidationError, ValidationResult, length};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchAction {
    Add,
    Update,
    Delete,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchEntry {
    pub path: String,
    pub action: PatchAction,
    #[serde(default)]
    pub destination: Option<String>,
    pub added_lines: u64,
    pub removed_lines: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    Read,
    Verify,
    Network,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDescriptor {
    pub command_id: String,
    pub display: String,
    pub source: String,
    pub risk_tier: RiskTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffFile {
    pub path: String,
    pub added: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageEntry {
    pub path: String,
    pub media_type: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointEntry {
    pub checkpoint_id: String,
    pub created_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchMatch {
    pub path: String,
    pub line: u64,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreOutput {
    ProjectRules {
        rules: Vec<RuleFile>,
    },
    ProjectStatus {
        branch: String,
        dirty_files: Vec<String>,
        staged_files: Vec<String>,
        rule_files: Vec<String>,
        command_ids: Vec<String>,
    },
    CodeSearch {
        matches: Vec<SearchMatch>,
    },
    FileApplyPatch {
        applied: Vec<PatchEntry>,
        checkpoint_id: String,
    },
    FileCreate {
        path: String,
        sha256: String,
        bytes_written: u64,
        checkpoint_id: String,
    },
    CommandList {
        commands: Vec<CommandDescriptor>,
    },
    CommandRun {
        command_id: String,
        exit_code: i64,
        stdout: String,
        stderr: String,
        duration_ms: u64,
        truncated: bool,
    },
    RepoStatus {
        branch: String,
        dirty_files: Vec<String>,
        staged_files: Vec<String>,
        remotes: Vec<String>,
        #[serde(default)]
        upstream: Option<String>,
        ahead: u64,
        behind: u64,
    },
    RepoDiff {
        files: Vec<DiffFile>,
        summary: String,
        patch: String,
        truncated: bool,
    },
    GitCommit {
        commit: String,
        branch: String,
        staged_files: Vec<String>,
    },
    GitPush {
        remote: String,
        branch: String,
        output: String,
    },
    ImageSave {
        image: ImageEntry,
        sha256: String,
    },
    ImageList {
        images: Vec<ImageEntry>,
    },
    ImageRetrieve {
        image: ImageEntry,
        data_base64: String,
    },
    CheckpointList {
        checkpoints: Vec<CheckpointEntry>,
    },
    CheckpointShow {
        checkpoint: CheckpointEntry,
        patch: String,
    },
    CheckpointRestore {
        checkpoint_id: String,
        restored_files: Vec<String>,
    },
}

impl Validate for CoreOutput {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::CodeSearch { matches } => {
                if matches.iter().any(|item| item.line == 0) {
                    Err(ValidationError::new("line", "must be at least 1"))
                } else {
                    Ok(())
                }
            }
            Self::ImageRetrieve { data_base64, .. } => {
                length("data_base64", data_base64, 0, usize::MAX)
            }
            _ => Ok(()),
        }
    }
}
