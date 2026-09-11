use cdr_runtime::soak::native_fixture;

use cdr_app_server::ResidentAppServer;
use std::path::Path;

pub async fn start(temp: &tempfile::TempDir, state: &Path, scenario: &str) -> ResidentAppServer {
    let mut config = native_fixture::config("archive");
    config
        .environment
        .insert("ARCHIVE_STATE".into(), state.to_string_lossy().into_owned());
    config.environment.insert(
        "ARCHIVE_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    config
        .environment
        .insert("ARCHIVE_SCENARIO".into(), scenario.into());
    ResidentAppServer::start(config).await.unwrap()
}

pub fn calls(temp: &tempfile::TempDir) -> Vec<serde_json::Value> {
    std::fs::read(temp.path().join("rpc.jsonl"))
        .unwrap()
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| line.ends_with(b"\n"))
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}
