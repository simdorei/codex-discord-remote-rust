use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::oauth_store::{OAuthStoreError, OAuthTokenRecord};

#[derive(Clone)]
pub struct OAuthProviderConfig {
    pub public_base_url: Url,
    pub owner_token: String,
    pub access_token_seconds: i64,
    pub refresh_token_seconds: i64,
    pub pending_authorization_limit: usize,
    pub authorization_code_limit: usize,
    pub authorization_code_per_client_limit: usize,
}

impl Default for OAuthProviderConfig {
    fn default() -> Self {
        Self {
            public_base_url: Url::parse("http://127.0.0.1").expect("static URL is valid"),
            owner_token: String::new(),
            access_token_seconds: 3_600,
            refresh_token_seconds: 60 * 60 * 24 * 30,
            pending_authorization_limit: 100,
            authorization_code_limit: 1_024,
            authorization_code_per_client_limit: 64,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub scopes: Vec<String>,
    pub resource: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingApproval {
    pub request_id: String,
    pub approval_url: Url,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeExchangeRequest {
    pub client_id: String,
    pub client_secret: String,
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
    pub resource: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefreshExchangeRequest {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub scopes: Vec<String>,
    pub resource: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub scope: String,
    pub refresh_token: String,
}

#[derive(Debug, Error)]
pub enum OAuthProviderError {
    #[error("{0}")]
    Configuration(&'static str),
    #[error("{0}")]
    InvalidRequest(String),
    #[error("OAuth client authentication failed.")]
    InvalidClient,
    #[error("Authorization grant is no longer valid.")]
    InvalidGrant,
    #[error("Requested OAuth scope is not allowed.")]
    InvalidScope,
    #[error("OAuth grant type is not supported.")]
    UnsupportedGrantType,
    #[error("OAuth storage capacity is temporarily unavailable.")]
    TemporarilyUnavailable,
    #[error("Authorization request expired.")]
    ApprovalNotFound,
    #[error("Owner token did not match.")]
    ApprovalDenied,
    #[error("OAuth storage task failed: {0}")]
    StorageTask(String),
    #[error(transparent)]
    Store(#[from] OAuthStoreError),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct StoredClient {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uris: Vec<String>,
    pub scope: String,
}

#[derive(Clone)]
pub(super) struct PendingAuthorization {
    pub request: AuthorizationRequest,
    pub scopes: Vec<String>,
    pub expires_at: i64,
    pub failed_attempts: u8,
}

#[derive(Clone)]
pub(super) struct AuthorizationCodeRecord {
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub scopes: Vec<String>,
    pub resource: String,
    pub expires_at: i64,
}

#[derive(Default)]
pub(super) struct OAuthState {
    pub pending: HashMap<String, PendingAuthorization>,
    pub codes: HashMap<String, AuthorizationCodeRecord>,
    pub exchanging: HashMap<String, AuthorizationCodeRecord>,
}

impl OAuthState {
    pub fn discard_expired(&mut self, now: i64) {
        self.pending.retain(|_, item| item.expires_at >= now);
        self.codes.retain(|_, item| item.expires_at >= now);
        self.exchanging.retain(|_, item| item.expires_at >= now);
    }
}

pub(super) fn token_response(
    access: OAuthTokenRecord,
    refresh: OAuthTokenRecord,
    access_token_seconds: i64,
) -> OAuthTokenResponse {
    OAuthTokenResponse {
        access_token: access.token,
        token_type: "Bearer",
        expires_in: access_token_seconds,
        scope: access.scopes.join(" "),
        refresh_token: refresh.token,
    }
}
