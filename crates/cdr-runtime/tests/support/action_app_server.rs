use cdr_runtime::soak::native_fixture;

use std::fs;
use std::path::Path;

use cdr_app_server::ResidentAppServer;
use serde_json::Value;

#[allow(dead_code)]
pub async fn start_fake_server(temp: &tempfile::TempDir, log: &Path) -> ResidentAppServer {
    start_server(temp, log, None).await
}

#[allow(dead_code)]
pub async fn start_persisting_server(
    temp: &tempfile::TempDir,
    log: &Path,
    state: &Path,
) -> ResidentAppServer {
    start_server(temp, log, Some(state)).await
}

async fn start_server(
    _temp: &tempfile::TempDir,
    log: &Path,
    state: Option<&Path>,
) -> ResidentAppServer {
    let mut config = native_fixture::config("action");
    if let Some(state) = state {
        config.environment.insert(
            "CDR_NEW_STATE_FIXTURE".into(),
            state.to_string_lossy().into_owned(),
        );
    }
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        log.to_string_lossy().into_owned(),
    );
    ResidentAppServer::start(config).await.unwrap()
}

pub fn rpc_log(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
