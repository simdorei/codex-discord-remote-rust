use cdr_runtime::soak::native_fixture;

use cdr_app_server::ResidentAppServer;
use std::path::Path;

pub async fn start(_root: &Path, log: &Path, mode: &str) -> ResidentAppServer {
    let mut config = native_fixture::config("usage");
    config
        .environment
        .insert("USAGE_TEST_MODE".into(), mode.into());
    config
        .environment
        .insert("USAGE_TEST_LOG".into(), log.to_string_lossy().into_owned());
    ResidentAppServer::start(config).await.unwrap()
}
