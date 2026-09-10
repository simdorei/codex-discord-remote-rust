use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use cdr_mcp_server::mcp_http::{DispatchOutput, ToolCallContext, ToolDispatcher, mcp_router};
use cdr_mcp_server::oauth::{OAuthProvider, OAuthProviderConfig};
use cdr_mcp_server::oauth_store::{OAuthStore, OAuthStoreLimits, OAuthTokenRecord};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn oauth_protected_streamable_http_lists_and_dispatches_all_tools() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let store = Arc::new(
        OAuthStore::open(
            &temp.path().join("oauth.sqlite3"),
            OAuthStoreLimits::default(),
        )
        .expect("open OAuth store"),
    );
    store
        .save_token_pair(
            &OAuthTokenRecord {
                token: "mcp-access".to_owned(),
                client_id: "client-a".to_owned(),
                scopes: vec!["files:read".to_owned(), "files:write".to_owned()],
                expires_at: Some(4_000_000_000),
                resource: Some("https://example.test/mcp".to_owned()),
                subject: Some("owner".to_owned()),
            },
            &OAuthTokenRecord {
                token: "mcp-refresh".to_owned(),
                client_id: "client-a".to_owned(),
                scopes: vec!["files:read".to_owned(), "files:write".to_owned()],
                expires_at: Some(4_000_000_000),
                resource: None,
                subject: Some("owner".to_owned()),
            },
            "family-a",
        )
        .expect("seed OAuth pair");
    let provider = OAuthProvider::new(
        store,
        OAuthProviderConfig {
            public_base_url: "https://example.test".parse().expect("public URL"),
            owner_token: "owner-secret-12345678901234567890".to_owned(),
            ..OAuthProviderConfig::default()
        },
    )
    .expect("create OAuth provider");
    let shutdown = CancellationToken::new();
    let app = mcp_router(&provider, Arc::new(EchoDispatcher), shutdown.clone())
        .expect("build MCP router");
    let (client, base) = spawn(app, shutdown.clone()).await;
    let headers = [
        ("Accept", "application/json, text/event-stream"),
        ("Content-Type", "application/json"),
        ("Authorization", "Bearer mcp-access"),
    ];
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "integration-test", "version": "1"}
        }
    });
    let unauthorized = client
        .post(format!("{base}/mcp"))
        .json(&initialize)
        .send()
        .await
        .expect("send unauthorized initialize");
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(unauthorized.headers().contains_key("www-authenticate"));
    let bad_origin = client
        .post(format!("{base}/mcp"))
        .header("Origin", "https://attacker.example")
        .header("Authorization", "Bearer mcp-access")
        .json(&initialize)
        .send()
        .await
        .expect("send cross-origin initialize");
    assert!(bad_origin.status().is_client_error());

    let initialized = post(&client, &base, &headers, initialize).await;
    assert_eq!(
        initialized["result"]["serverInfo"]["name"],
        "simdorei-local-project"
    );
    let listed = post(
        &client,
        &base,
        &headers,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    let tools = listed["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 51);
    assert!(tools.iter().any(|tool| tool["name"] == "repo_status"));
    let python_inventory: Value =
        serde_json::from_str(include_str!("../assets/python_tool_inventory.json"))
            .expect("decode Python tool inventory");
    assert_eq!(Value::Array(tools.clone()), python_inventory);

    let called = post(
        &client,
        &base,
        &headers,
        json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{
                "name":"repo_status",
                "arguments":{"sample":7},
                "_meta":{"openai/session":"chat-session-a"}
            }
        }),
    )
    .await;
    assert_eq!(called["result"]["structuredContent"]["tool"], "repo_status");
    assert_eq!(
        called["result"]["structuredContent"]["session"],
        "chat-session-a"
    );
    shutdown.cancel();
}

struct EchoDispatcher;

impl ToolDispatcher for EchoDispatcher {
    fn dispatch(
        &self,
        context: ToolCallContext,
        tool: String,
        arguments: serde_json::Map<String, Value>,
    ) -> Pin<Box<dyn Future<Output = Result<DispatchOutput, String>> + Send>> {
        Box::pin(async move {
            Ok(DispatchOutput::Structured(json!({
                "tool": tool,
                "session": context.session,
                "arguments": arguments,
            })))
        })
    }
}

async fn post(
    client: &reqwest::Client,
    base: &str,
    headers: &[(&str, &str)],
    body: Value,
) -> Value {
    let mut request = client.post(format!("{base}/mcp")).json(&body);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await.expect("send MCP request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.expect("decode MCP response")
}

async fn spawn(app: axum::Router, shutdown: CancellationToken) -> (reqwest::Client, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .expect("serve MCP test app");
    });
    (reqwest::Client::new(), format!("http://{address}"))
}
