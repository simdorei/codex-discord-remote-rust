use cdr_core::{Validate, ValidationResult, length};
use serde::{Deserialize, Serialize};

use super::{ListFilesOutput, ProjectInfoOutput, ReadFileOutput, WriteFileOutput};
use crate::output::ProjectOperationOutput;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BridgeResult {
    ProjectInfoResult {
        request_id: String,
        output: ProjectInfoOutput,
    },
    ListFilesResult {
        request_id: String,
        output: ListFilesOutput,
    },
    ReadFileResult {
        request_id: String,
        output: ReadFileOutput,
    },
    WriteFileResult {
        request_id: String,
        output: WriteFileOutput,
    },
    ProjectOperationResult {
        request_id: String,
        output: ProjectOperationOutput,
    },
    ProjectSessionResult {
        request_id: String,
    },
    OperationError {
        request_id: String,
        error_code: String,
        message: String,
    },
}

impl Validate for BridgeResult {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::ProjectInfoResult { output, .. } => output.validate(),
            Self::ListFilesResult { output, .. } => output.validate(),
            Self::ReadFileResult { output, .. } => output.validate(),
            Self::WriteFileResult { output, .. } => output.validate(),
            Self::ProjectOperationResult { output, .. } => output.validate(),
            Self::OperationError {
                error_code,
                message,
                ..
            } => {
                length("error_code", error_code, 1, 100)?;
                length("message", message, 1, 1_000)
            }
            Self::ProjectSessionResult { .. } => Ok(()),
        }
    }
}
