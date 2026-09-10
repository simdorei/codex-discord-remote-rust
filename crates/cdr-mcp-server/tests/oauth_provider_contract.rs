use std::sync::Arc;

use cdr_mcp_server::oauth::{
    AuthorizationRequest, CodeExchangeRequest, OAuthProvider, OAuthProviderConfig,
    RefreshExchangeRequest,
};
use cdr_mcp_server::oauth_store::{OAuthStore, OAuthStoreLimits};
use serde_json::json;
use sha2::{Digest, Sha256};

const BASE_URL: &str = "https://example.test";
const RESOURCE: &str = "https://example.test/mcp";
const OWNER_TOKEN: &str = "owner-secret-12345678901234567890";

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn dcr_pkce_code_refresh_replay_and_restart_contract() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("oauth.sqlite3");
    let first_provider = provider(&database);
    let registered = first_provider
        .register_client(json!({
            "redirect_uris": ["https://chatgpt.com/connector/oauth/test"],
            "token_endpoint_auth_method": "client_secret_post",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "client_name": "Rust OAuth contract"
        }))
        .await
        .expect("register client");
    let client_id = registered["client_id"]
        .as_str()
        .expect("client id")
        .to_owned();
    let client_secret = registered["client_secret"]
        .as_str()
        .expect("client secret")
        .to_owned();

    drop(first_provider);
    let restarted = provider(&database);
    let verifier = "oauth-pkce-verifier-123456789012345678901234567890";
    let challenge = pkce_challenge(verifier);
    let approval = restarted
        .begin_authorization(AuthorizationRequest {
            client_id: client_id.clone(),
            redirect_uri: "https://chatgpt.com/connector/oauth/test".to_owned(),
            state: Some("state-a".to_owned()),
            code_challenge: challenge,
            code_challenge_method: "S256".to_owned(),
            scopes: vec!["files:read".to_owned(), "files:write".to_owned()],
            resource: RESOURCE.to_owned(),
        })
        .await
        .expect("start authorization");
    let denied = restarted
        .begin_authorization(AuthorizationRequest {
            client_id: client_id.clone(),
            redirect_uri: "https://chatgpt.com/connector/oauth/test".to_owned(),
            state: None,
            code_challenge: pkce_challenge(verifier),
            code_challenge_method: "S256".to_owned(),
            scopes: vec!["files:read".to_owned()],
            resource: RESOURCE.to_owned(),
        })
        .await
        .expect("start denied authorization");
    for _ in 0..5 {
        assert!(
            restarted
                .approve(&denied.request_id, "wrong-owner-token")
                .await
                .is_err()
        );
    }
    assert_eq!(
        restarted
            .pending_scopes(&denied.request_id)
            .await
            .expect("inspect denied authorization"),
        None
    );
    assert!(approval.approval_url.as_str().contains("/oauth/approve?"));
    let callback = restarted
        .approve(&approval.request_id, OWNER_TOKEN)
        .await
        .expect("approve authorization");
    assert_eq!(
        callback
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned()),
        Some("state-a".to_owned())
    );
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .expect("authorization code");
    let first = restarted
        .exchange_code(CodeExchangeRequest {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            code,
            redirect_uri: "https://chatgpt.com/connector/oauth/test".to_owned(),
            code_verifier: verifier.to_owned(),
            resource: RESOURCE.to_owned(),
        })
        .await
        .expect("exchange authorization code");
    let successor = restarted
        .exchange_refresh(RefreshExchangeRequest {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            refresh_token: first.refresh_token.clone(),
            scopes: vec!["files:read".to_owned()],
            resource: RESOURCE.to_owned(),
        })
        .await
        .expect("rotate refresh token");
    assert!(
        restarted
            .authenticate_access(&successor.access_token)
            .await
            .expect("authenticate successor")
            .is_some()
    );

    let replay = restarted
        .exchange_refresh(RefreshExchangeRequest {
            client_id,
            client_secret,
            refresh_token: first.refresh_token,
            scopes: vec!["files:read".to_owned()],
            resource: RESOURCE.to_owned(),
        })
        .await;
    assert!(replay.is_err(), "spent refresh must not be reusable");
    assert!(
        restarted
            .authenticate_access(&successor.access_token)
            .await
            .expect("check revoked successor")
            .is_none()
    );
}

fn provider(path: &std::path::Path) -> OAuthProvider {
    let store = OAuthStore::open(path, OAuthStoreLimits::default()).expect("open OAuth store");
    OAuthProvider::new(
        Arc::new(store),
        OAuthProviderConfig {
            public_base_url: BASE_URL.parse().expect("base URL"),
            owner_token: OWNER_TOKEN.to_owned(),
            ..OAuthProviderConfig::default()
        },
    )
    .expect("create OAuth provider")
}

fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine as _;

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
