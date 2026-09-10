use std::time::Duration;

use cdr_app_server::{AppServerClient, AppServerConfig};
use serde_json::json;

fn fake_config() -> AppServerConfig {
    let mut config = AppServerConfig::new(env!("CARGO_BIN_EXE_fake_codex_app_server"));
    config.arguments.clear();
    config
}

#[tokio::test]
async fn configured_environment_is_passed_to_the_app_server_child() {
    let config = fake_config().with_environment([(
        "CDR_TEST_CHILD_ENV".to_owned(),
        "from-runtime-config".to_owned(),
    )]);
    let client = AppServerClient::start(config)
        .await
        .expect("start fake app-server");

    let result = client
        .request(
            "test/env",
            json!({"name":"CDR_TEST_CHILD_ENV"}),
            Duration::from_secs(1),
        )
        .await
        .expect("read child environment");

    assert_eq!(result, json!({"value":"from-runtime-config"}));
    client.close().await.expect("close app-server");
}
