use cdr_runtime::soak::native_fixture;

use cdr_app_server::ResidentAppServer;
use std::path::Path;

pub async fn start(_temp: &tempfile::TempDir, log: &Path, scenario: &str) -> ResidentAppServer {
    let mut config = native_fixture::config("resume");
    config
        .environment
        .insert("RESUME_SCENARIO".into(), scenario.into());
    config
        .environment
        .insert("RESUME_LOG".into(), log.to_string_lossy().into_owned());
    ResidentAppServer::start(config).await.unwrap()
}

pub fn calls(log: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
