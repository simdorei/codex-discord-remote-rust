use cdr_runtime::soak::native_fixture;

use cdr_app_server::ResidentAppServer;
use std::path::Path;

pub async fn start(root: &Path, mode: &str) -> ResidentAppServer {
    let mut config = native_fixture::config("display");
    config
        .environment
        .insert("DISPLAY_MODE".into(), mode.into());
    config.environment.insert(
        "DISPLAY_LOG".into(),
        root.join("display-rpc.jsonl")
            .to_string_lossy()
            .into_owned(),
    );
    ResidentAppServer::start(config).await.unwrap()
}
