mod authorization;
mod clients;
mod model;
mod token_exchange;
mod util;

use std::sync::Arc;

pub use model::{
    AuthorizationRequest, CodeExchangeRequest, OAuthProviderConfig, OAuthProviderError,
    OAuthTokenResponse, PendingApproval, RefreshExchangeRequest,
};
use model::{OAuthState, StoredClient};
use tokio::sync::Mutex;
use url::Url;

use crate::oauth_store::{OAuthStore, OAuthTokenRecord};

#[derive(Clone)]
pub struct OAuthProvider {
    store: Arc<OAuthStore>,
    config: Arc<OAuthProviderConfig>,
    state: Arc<Mutex<OAuthState>>,
    resource_url: Arc<Url>,
}

impl OAuthProvider {
    pub fn new(
        store: Arc<OAuthStore>,
        config: OAuthProviderConfig,
    ) -> Result<Self, OAuthProviderError> {
        validate_config(&config)?;
        let resource_url = util::endpoint(&config.public_base_url, "/mcp")?;
        Ok(Self {
            store,
            config: Arc::new(config),
            state: Arc::new(Mutex::new(OAuthState::default())),
            resource_url: Arc::new(resource_url),
        })
    }

    #[must_use]
    pub fn public_base_url(&self) -> &Url {
        &self.config.public_base_url
    }

    #[must_use]
    pub fn resource_url(&self) -> &Url {
        &self.resource_url
    }

    async fn load_client(
        &self,
        client_id: &str,
    ) -> Result<Option<StoredClient>, OAuthProviderError> {
        let store = self.store.clone();
        let client_id = client_id.to_owned();
        let payload = tokio::task::spawn_blocking(move || store.get_client_payload(&client_id))
            .await
            .map_err(|error| OAuthProviderError::StorageTask(error.to_string()))??;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(OAuthProviderError::from))
            .transpose()
    }

    async fn store_access(
        &self,
        token: String,
    ) -> Result<Option<OAuthTokenRecord>, OAuthProviderError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.load_access_token(&token))
            .await
            .map_err(|error| OAuthProviderError::StorageTask(error.to_string()))?
            .map_err(OAuthProviderError::from)
    }
}

fn validate_config(config: &OAuthProviderConfig) -> Result<(), OAuthProviderError> {
    if !matches!(config.public_base_url.scheme(), "http" | "https")
        || config.public_base_url.host_str().is_none()
    {
        return Err(OAuthProviderError::Configuration(
            "OAuth public base URL must be HTTP(S) with a host.",
        ));
    }
    if config.owner_token.len() < 24 {
        return Err(OAuthProviderError::Configuration(
            "OAuth owner token must contain at least 24 characters.",
        ));
    }
    if config.access_token_seconds < 300 || config.refresh_token_seconds < 3_600 {
        return Err(OAuthProviderError::Configuration(
            "OAuth token lifetimes are below the supported minimum.",
        ));
    }
    if config.pending_authorization_limit == 0
        || config.authorization_code_limit == 0
        || config.authorization_code_per_client_limit == 0
        || config.authorization_code_per_client_limit > config.authorization_code_limit
    {
        return Err(OAuthProviderError::Configuration(
            "OAuth in-memory capacity limits are invalid.",
        ));
    }
    Ok(())
}
