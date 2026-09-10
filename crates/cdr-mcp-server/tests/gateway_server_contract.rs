use std::collections::HashMap;

use cdr_mcp_server::server::{GatewayConfig, gateway_router};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn python_compatible_environment_builds_one_healthy_gateway() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let values = HashMap::from([
        (
            "SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON".into(),
            serde_json::json!({
                "version":1,
                "devices":[{
                    "device_id":"windows-a",
                    "token":"a-device-token-with-at-least-32-bytes"
                }]
            })
            .to_string(),
        ),
        (
            "SIMDOREI_MCP_PUBLIC_BASE_URL".into(),
            "https://simdorei.duckdns.org".into(),
        ),
        (
            "SIMDOREI_MCP_OWNER_TOKEN".into(),
            "an-owner-secret-with-at-least-24-bytes".into(),
        ),
        (
            "SIMDOREI_MCP_OAUTH_DATABASE_PATH".into(),
            temp.path().join("oauth.sqlite3").display().to_string(),
        ),
    ]);
    let config = GatewayConfig::from_map(&values).expect("load compatible settings");
    let shutdown = CancellationToken::new();
    let app = gateway_router(config, shutdown.clone()).expect("build gateway");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway");
    let address = listener.local_addr().expect("gateway address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .expect("serve gateway");
    });
    let response: serde_json::Value = reqwest::get(format!("http://{address}/healthz"))
        .await
        .expect("request health")
        .error_for_status()
        .expect("healthy status")
        .json()
        .await
        .expect("health JSON");
    assert_eq!(response["ok"], true);
    assert_eq!(response["service"], "simdorei-local-project-mcp");
    assert_eq!(response["configured_devices"], 1);
    assert_eq!(response["connected_devices"], 0);
}
