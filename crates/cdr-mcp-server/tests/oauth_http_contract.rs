use std::sync::Arc;

use cdr_mcp_server::oauth::{OAuthProvider, OAuthProviderConfig};
use cdr_mcp_server::oauth_http::oauth_router;
use cdr_mcp_server::oauth_store::{OAuthStore, OAuthStoreLimits};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn oauth_http_discovery_dcr_pkce_and_revocation_contract() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let store = OAuthStore::open(
        &temp.path().join("oauth.sqlite3"),
        OAuthStoreLimits::default(),
    )
    .expect("open OAuth store");
    let provider = OAuthProvider::new(
        Arc::new(store),
        OAuthProviderConfig {
            public_base_url: "https://example.test".parse().expect("public URL"),
            owner_token: "owner-secret-12345678901234567890".to_owned(),
            ..OAuthProviderConfig::default()
        },
    )
    .expect("create provider");
    let (client, base, shutdown) = spawn(oauth_router(provider)).await;

    let unsupported: Value = client
        .post(format!("{base}/token"))
        .form(&[("grant_type", "device_code")])
        .send()
        .await
        .expect("send unsupported grant")
        .json()
        .await
        .expect("decode unsupported grant error");
    assert_eq!(unsupported["error"], "unsupported_grant_type");

    let protected: Value = client
        .get(format!("{base}/.well-known/oauth-protected-resource/mcp"))
        .send()
        .await
        .expect("fetch protected-resource metadata")
        .json()
        .await
        .expect("decode protected-resource metadata");
    assert_eq!(protected["resource"], "https://example.test/mcp");
    let authorization: Value = client
        .get(format!("{base}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .expect("fetch authorization metadata")
        .json()
        .await
        .expect("decode authorization metadata");
    assert_eq!(
        authorization["registration_endpoint"],
        "https://example.test/register"
    );
    assert_eq!(
        authorization["code_challenge_methods_supported"],
        json!(["S256"])
    );

    let registered: Value = client
        .post(format!("{base}/register"))
        .json(&json!({
            "redirect_uris": ["https://chatgpt.com/connector/oauth/test"],
            "token_endpoint_auth_method": "client_secret_post",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"]
        }))
        .send()
        .await
        .expect("register client")
        .error_for_status()
        .expect("registration succeeds")
        .json()
        .await
        .expect("decode registration");
    let verifier = "oauth-pkce-verifier-123456789012345678901234567890";
    let authorization = client
        .get(format!("{base}/authorize"))
        .query(&[
            ("response_type", "code"),
            (
                "client_id",
                registered["client_id"].as_str().expect("client id"),
            ),
            ("redirect_uri", "https://chatgpt.com/connector/oauth/test"),
            ("state", "http-state"),
            ("code_challenge", &pkce_challenge(verifier)),
            ("code_challenge_method", "S256"),
            ("scope", "files:read files:write"),
            ("resource", "https://example.test/mcp"),
        ])
        .send()
        .await
        .expect("start authorization");
    assert_eq!(authorization.status(), reqwest::StatusCode::FOUND);
    let approval = authorization
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("approval location")
        .to_str()
        .expect("location text");
    let request_id = url::Url::parse(approval)
        .expect("approval URL")
        .query_pairs()
        .find(|(key, _)| key == "request_id")
        .map(|(_, value)| value.into_owned())
        .expect("request id");
    let approved = client
        .post(format!("{base}/oauth/approve"))
        .form(&[
            ("request_id", request_id.as_str()),
            ("owner_token", "owner-secret-12345678901234567890"),
        ])
        .send()
        .await
        .expect("approve request");
    assert_eq!(approved.status(), reqwest::StatusCode::FOUND);
    let callback = url::Url::parse(
        approved
            .headers()
            .get(reqwest::header::LOCATION)
            .expect("callback location")
            .to_str()
            .expect("callback text"),
    )
    .expect("callback URL");
    assert_eq!(
        callback
            .query_pairs()
            .find(|(key, _)| key == "iss")
            .map(|(_, value)| value.into_owned()),
        Some("https://example.test".to_owned())
    );
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .expect("authorization code");
    let token: Value = client
        .post(format!("{base}/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            (
                "client_id",
                registered["client_id"].as_str().expect("client id"),
            ),
            (
                "client_secret",
                registered["client_secret"].as_str().expect("client secret"),
            ),
            ("redirect_uri", "https://chatgpt.com/connector/oauth/test"),
            ("code_verifier", verifier),
            ("resource", "https://example.test/mcp"),
        ])
        .send()
        .await
        .expect("exchange code")
        .error_for_status()
        .expect("token exchange succeeds")
        .json()
        .await
        .expect("decode token response");
    assert_eq!(token["token_type"], "Bearer");
    let revoked = client
        .post(format!("{base}/revoke"))
        .form(&[
            (
                "token",
                token["access_token"].as_str().expect("access token"),
            ),
            (
                "client_id",
                registered["client_id"].as_str().expect("client id"),
            ),
            (
                "client_secret",
                registered["client_secret"].as_str().expect("client secret"),
            ),
        ])
        .send()
        .await
        .expect("revoke token");
    assert_eq!(revoked.status(), reqwest::StatusCode::OK);
    shutdown.cancel();
}

async fn spawn(app: axum::Router) -> (reqwest::Client, String, CancellationToken) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let shutdown = CancellationToken::new();
    let stop = shutdown.clone();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
            .expect("serve OAuth test app");
    });
    (
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("build HTTP client"),
        format!("http://{address}"),
        shutdown,
    )
}

fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine as _;

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
