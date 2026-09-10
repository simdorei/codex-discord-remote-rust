use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SelectProjectArgs {
    pub project_scope: String,
    pub connector_resource: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SelectDeviceArgs {
    pub device_id: String,
    pub working_directory: String,
    pub connector_resource: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DirectoryArgs {
    pub working_directory: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListFilesArgs {
    #[serde(default = "default_pattern")]
    pub pattern: String,
    #[serde(default = "default_limit")]
    pub limit: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadFileArgs {
    pub path: String,
    #[serde(default = "default_start_line")]
    pub start_line: u64,
    #[serde(default = "default_max_lines")]
    pub max_lines: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WriteFileArgs {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

fn default_pattern() -> String {
    "**/*".into()
}
fn default_limit() -> u16 {
    200
}
fn default_start_line() -> u64 {
    1
}
fn default_max_lines() -> u16 {
    250
}
