use cdr_core::{Validate, ValidationError, ValidationResult, length};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectInfoOutput {
    pub root: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFilesOutput {
    pub files: Vec<FileEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileOutput {
    pub path: String,
    pub content: String,
    pub sha256: String,
    pub start_line: u64,
    pub end_line: u64,
    pub total_lines: u64,
    pub truncated: bool,
    pub redacted: bool,
}

impl Validate for ReadFileOutput {
    fn validate(&self) -> ValidationResult {
        if self.start_line == 0 {
            Err(ValidationError::new("start_line", "must be at least 1"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteFileOutput {
    pub path: String,
    pub sha256: String,
    pub bytes_written: u64,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSummary {
    pub device_id: String,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceListOutput {
    pub devices: Vec<DeviceSummary>,
}

impl Validate for ProjectInfoOutput {
    fn validate(&self) -> ValidationResult {
        Ok(())
    }
}
impl Validate for ListFilesOutput {
    fn validate(&self) -> ValidationResult {
        Ok(())
    }
}
impl Validate for WriteFileOutput {
    fn validate(&self) -> ValidationResult {
        length("sha256", &self.sha256, 0, usize::MAX)
    }
}
