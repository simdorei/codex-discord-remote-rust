use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use super::responses::endpoint_text;
use crate::oauth::OAuthProvider;
use crate::scopes::ALL;

pub(super) async fn protected_resource(State(provider): State<OAuthProvider>) -> Json<Value> {
    Json(json!({
        "resource": provider.resource_url().as_str(),
        "authorization_servers": [provider.public_base_url().as_str().trim_end_matches('/')],
        "scopes_supported": ALL,
        "bearer_methods_supported": ["header"]
    }))
}

pub(super) async fn authorization_server(State(provider): State<OAuthProvider>) -> Json<Value> {
    let base = provider.public_base_url();
    Json(json!({
        "issuer": base.as_str().trim_end_matches('/'),
        "authorization_endpoint": endpoint_text(base, "/authorize"),
        "token_endpoint": endpoint_text(base, "/token"),
        "registration_endpoint": endpoint_text(base, "/register"),
        "revocation_endpoint": endpoint_text(base, "/revoke"),
        "scopes_supported": ALL,
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
        "authorization_response_iss_parameter_supported": true
    }))
}
