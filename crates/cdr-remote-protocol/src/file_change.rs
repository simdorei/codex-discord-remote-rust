use cdr_core::{Validate, ValidationError, ValidationResult, count, length};
use serde::{Deserialize, Serialize};

use crate::identifiers::validate_sha256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeAction {
    Create,
    Update,
    Delete,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileChange {
    pub action: FileChangeAction,
    pub path: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

impl Validate for FileChange {
    fn validate(&self) -> ValidationResult {
        length("path", &self.path, 1, 1_000)?;
        if let Some(content) = &self.content {
            length("content", content, 0, 1_048_576)?;
        }
        if let Some(destination) = &self.destination {
            length("destination", destination, 1, 1_000)?;
        }
        if let Some(digest) = &self.expected_sha256 {
            validate_sha256("expected_sha256", digest)?;
        }
        match self.action {
            FileChangeAction::Create if self.content.is_none() => {
                return Err(ValidationError::new("content", "create requires content"));
            }
            FileChangeAction::Update if self.content.is_none() => {
                return Err(ValidationError::new("content", "update requires content"));
            }
            FileChangeAction::Update | FileChangeAction::Delete | FileChangeAction::Move
                if self.expected_sha256.is_none() =>
            {
                return Err(ValidationError::new(
                    "expected_sha256",
                    "operation requires expected_sha256",
                ));
            }
            FileChangeAction::Move if self.destination.is_none() => {
                return Err(ValidationError::new(
                    "destination",
                    "move requires destination",
                ));
            }
            FileChangeAction::Create if self.expected_sha256.is_some() => {
                return Err(ValidationError::new(
                    "expected_sha256",
                    "create does not accept expected_sha256",
                ));
            }
            FileChangeAction::Delete if self.content.is_some() => {
                return Err(ValidationError::new(
                    "content",
                    "delete does not accept content",
                ));
            }
            _ => {}
        }
        if !matches!(self.action, FileChangeAction::Move) && self.destination.is_some() {
            return Err(ValidationError::new(
                "destination",
                "destination is only valid for move",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileApplyPatchRequest {
    pub changes: Vec<FileChange>,
}

impl Validate for FileApplyPatchRequest {
    fn validate(&self) -> ValidationResult {
        count("changes", self.changes.len(), 1, 200)?;
        let mut content_size = 0_usize;
        for change in &self.changes {
            change.validate()?;
            content_size += change.content.as_ref().map_or(0, String::len);
        }
        if content_size > 10_485_760 {
            return Err(ValidationError::new(
                "changes",
                "combined file change content exceeds 10 MiB",
            ));
        }
        Ok(())
    }
}
