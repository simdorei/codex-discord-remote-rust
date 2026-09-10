use std::time::Duration;

use cdr_app_server::{AppServerClient, AppServerConfig};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "requires CDR_LIVE_CODEX_EXE and starts a real Codex app-server"]
async fn current_codex_app_server_initializes_and_lists_models() {
    let executable = std::env::var_os("CDR_LIVE_CODEX_EXE")
        .expect("CDR_LIVE_CODEX_EXE must point to the current codex executable");
    let client = AppServerClient::start(AppServerConfig::new(executable))
        .await
        .expect("initialize current Codex app-server");
    let result = client
        .request("model/list", json!({}), Duration::from_secs(10))
        .await
        .expect("model/list response");
    assert!(
        matches!(result, Value::Object(_)),
        "unexpected response: {result}"
    );
    assert!(client.lifecycle_snapshot().healthy);
    client
        .close()
        .await
        .expect("close current Codex app-server");
}
