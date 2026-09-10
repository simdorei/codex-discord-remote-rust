use std::path::PathBuf;

use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OAuthTokenRecord {
    pub token: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<i64>,
    pub resource: Option<String>,
    pub subject: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshRotationOutcome {
    Rotated,
    Missing,
    Replayed,
    HistoryExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OAuthStoreLimits {
    pub max_clients: i64,
    pub max_token_families: i64,
    pub max_token_families_per_client: i64,
    pub max_refresh_history_global: i64,
    pub max_refresh_history_per_family: i64,
}

impl Default for OAuthStoreLimits {
    fn default() -> Self {
        Self {
            max_clients: 500,
            max_token_families: 256,
            max_token_families_per_client: 16,
            max_refresh_history_global: 65_536,
            max_refresh_history_per_family: 1_024,
        }
    }
}

impl OAuthStoreLimits {
    pub(super) fn validate(self) -> Result<Self, OAuthStoreError> {
        if self.max_clients < 1 {
            return Err(OAuthStoreError::Configuration(
                "OAuth client limit must be positive.",
            ));
        }
        if self.max_token_families < 1 || self.max_token_families_per_client < 1 {
            return Err(OAuthStoreError::Configuration(
                "OAuth token family limits must be positive.",
            ));
        }
        if self.max_token_families_per_client > self.max_token_families {
            return Err(OAuthStoreError::Configuration(
                "The per-client token family limit cannot exceed the global limit.",
            ));
        }
        if self.max_refresh_history_global < 1 || self.max_refresh_history_per_family < 1 {
            return Err(OAuthStoreError::Configuration(
                "OAuth refresh history limits must be positive.",
            ));
        }
        if self.max_refresh_history_per_family > self.max_refresh_history_global {
            return Err(OAuthStoreError::Configuration(
                "The per-family refresh history limit cannot exceed the global limit.",
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Error)]
pub enum OAuthStoreError {
    #[error("{0}")]
    Configuration(&'static str),
    #[error("OAuth client storage is full with active registrations.")]
    ClientLimit,
    #[error("OAuth token family client limit reached.")]
    TokenFamilyClientLimit,
    #[error("OAuth token family global limit reached.")]
    TokenFamilyGlobalLimit,
    #[error("OAuth store lock was poisoned")]
    LockPoisoned,
    #[error("system clock is earlier than the Unix epoch")]
    InvalidSystemClock,
    #[error("could not create OAuth database directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
