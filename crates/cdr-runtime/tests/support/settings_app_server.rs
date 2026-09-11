use cdr_runtime::soak::native_fixture;

use cdr_app_server::ResidentAppServer;
use std::path::Path;

pub async fn start(_temp: &tempfile::TempDir, log: &Path, mode: &str) -> ResidentAppServer {
    let mut config = native_fixture::config("settings");
    config
        .environment
        .insert("SETTINGS_TEST_LOG".into(), log.to_string_lossy().into());
    config
        .environment
        .insert("SETTINGS_TEST_MODE".into(), mode.into());
    ResidentAppServer::start(config).await.unwrap()
}
