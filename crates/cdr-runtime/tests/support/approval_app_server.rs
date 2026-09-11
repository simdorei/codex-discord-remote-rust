use cdr_runtime::soak::native_fixture;

use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::path::Path;

pub async fn start(temp: &tempfile::TempDir, log: &Path) -> ResidentAppServer {
    ResidentAppServer::start(config(temp, log)).await.unwrap()
}

pub fn config(_temp: &tempfile::TempDir, log: &Path) -> AppServerConfig {
    let mut config = native_fixture::config("approval");
    config
        .environment
        .insert("CDR_APPROVAL_TEST_LOG".into(), log.to_string_lossy().into());
    config
}
