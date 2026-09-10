use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use cdr_mcp_server::bridge_http::{DeviceCredential, DeviceCredentialRegistry, bridge_router};
use cdr_mcp_server::broker::BridgeBroker;
use cdr_mcp_server::mcp_http::mcp_router;
use cdr_mcp_server::oauth::{OAuthProvider, OAuthProviderConfig};
use cdr_mcp_server::oauth_store::{OAuthStore, OAuthStoreLimits, OAuthTokenRecord};
use cdr_mcp_server::tool_dispatcher::{BrokerToolDispatcher, PRODUCTION_CONNECTOR_RESOURCE};
use cdr_remote_agent::bridge::connect_and_serve_until;
use cdr_remote_agent::config::load_remote_mcp_config;
use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const ACCESS_TOKEN: &str = "public-gateway-access";
const DEVICE_TOKEN: &str = "a-device-token-with-at-least-32-bytes";

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn oauth_mcp_and_real_agent_complete_a_public_file_read() {
    let temp = tempfile::tempdir().expect("temporary project");
    std::fs::write(temp.path().join("hello.txt"), "hello through public MCP")
        .expect("seed project file");
    let oauth = seeded_oauth(temp.path());
    let broker = BridgeBroker::new(Duration::from_secs(3));
    let credentials = DeviceCredentialRegistry::new(vec![
        DeviceCredential::new("windows-a", DEVICE_TOKEN).expect("device credential"),
    ])
    .expect("credential registry");
    let shutdown = CancellationToken::new();
    let dispatcher = Arc::new(BrokerToolDispatcher::new(
        broker.clone(),
        PRODUCTION_CONNECTOR_RESOURCE.to_owned(),
    ));
    let app = bridge_router(credentials, broker.clone())
        .merge(mcp_router(&oauth, dispatcher, shutdown.clone()).expect("build MCP router"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind public gateway");
    let address = listener.local_addr().expect("gateway address");
    let server_stop = shutdown.clone();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .expect("serve public gateway");
    });
    let config = load_remote_mcp_config(&HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            format!("ws://{address}/bridge"),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "windows-a".into()),
        ("CODEX_REMOTE_MCP_DEVICE_TOKEN".into(), DEVICE_TOKEN.into()),
    ]))
    .expect("agent config")
    .expect("enabled agent");
    let (stop_agent, stop_rx) = watch::channel(false);
    let agent = tokio::spawn(async move {
        connect_and_serve_until(&config, Arc::new(LocalProjectDispatcher::new()), 1, stop_rx).await
    });
    wait_for_device(&broker).await;

    let client = reqwest::Client::new();
    let base = format!("http://{address}");
    post(
        &client,
        &base,
        json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{
                "protocolVersion":"2025-11-25",
                "capabilities":{},
                "clientInfo":{"name":"public-e2e","version":"1"}
            }
        }),
    )
    .await;
    let selected = call(
        &client,
        &base,
        2,
        "select_device",
        json!({
            "device_id":"windows-a",
            "working_directory":temp.path().display().to_string(),
            "connector_resource":PRODUCTION_CONNECTOR_RESOURCE
        }),
    )
    .await;
    assert_eq!(
        selected["result"]["structuredContent"]["device_id"],
        "windows-a"
    );
    let read = call(
        &client,
        &base,
        3,
        "read_project_file",
        json!({"path":"hello.txt","start_line":1,"max_lines":20}),
    )
    .await;
    assert_eq!(
        read["result"]["structuredContent"]["content"],
        "hello through public MCP"
    );

    stop_agent.send_replace(true);
    agent.await.expect("agent task").expect("agent shutdown");
    shutdown.cancel();
}

fn seeded_oauth(path: &std::path::Path) -> OAuthProvider {
    let store = Arc::new(
        OAuthStore::open(&path.join("oauth.sqlite3"), OAuthStoreLimits::default())
            .expect("open OAuth store"),
    );
    store
        .save_token_pair(
            &token(ACCESS_TOKEN, Some(PRODUCTION_CONNECTOR_RESOURCE)),
            &token("public-gateway-refresh", None),
            "public-gateway-family",
        )
        .expect("seed OAuth pair");
    OAuthProvider::new(
        store,
        OAuthProviderConfig {
            public_base_url: "https://simdorei.duckdns.org".parse().expect("public URL"),
            owner_token: "owner-secret-12345678901234567890".to_owned(),
            ..OAuthProviderConfig::default()
        },
    )
    .expect("create OAuth provider")
}

fn token(value: &str, resource: Option<&str>) -> OAuthTokenRecord {
    OAuthTokenRecord {
        token: value.to_owned(),
        client_id: "public-e2e-client".to_owned(),
        scopes: vec!["files:read".to_owned(), "files:write".to_owned()],
        expires_at: Some(4_000_000_000),
        resource: resource.map(str::to_owned),
        subject: Some("owner".to_owned()),
    }
}

async fn call(
    client: &reqwest::Client,
    base: &str,
    id: u64,
    name: &str,
    arguments: Value,
) -> Value {
    post(
        client,
        base,
        json!({
            "jsonrpc":"2.0",
            "id":id,
            "method":"tools/call",
            "params":{
                "name":name,
                "arguments":arguments,
                "_meta":{"openai/session":"public-session-a"}
            }
        }),
    )
    .await
}

async fn post(client: &reqwest::Client, base: &str, body: Value) -> Value {
    let response = client
        .post(format!("{base}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {ACCESS_TOKEN}"))
        .json(&body)
        .send()
        .await
        .expect("send MCP request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.expect("decode MCP response")
}

async fn wait_for_device(broker: &BridgeBroker) {
    for _ in 0..50 {
        if !broker.list_devices().await.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Rust agent did not connect to the public gateway");
}
