use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use thiserror::Error;
use url::Url;

use crate::bridge_http::{DeviceCredential, DeviceCredentialError, DeviceCredentialRegistry};
use crate::oauth::OAuthProviderConfig;
use crate::oauth_store::OAuthStoreLimits;

const PREFIX: &str = "SIMDOREI_MCP_";

pub struct GatewayConfig {
    pub(crate) credentials: DeviceCredentialRegistry,
    pub(crate) oauth_database_path: PathBuf,
    pub(crate) oauth: OAuthProviderConfig,
    pub(crate) store_limits: OAuthStoreLimits,
    pub(crate) request_timeout_seconds: u64,
    bind_address: SocketAddr,
}

#[derive(Debug, Error)]
pub enum GatewayConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("legacy device ID and token must be set together")]
    PartialLegacyCredential,
    #[error("legacy device credentials must exactly match a registry entry")]
    LegacyCredentialMismatch,
    #[error("invalid gateway setting {name}: {message}")]
    Invalid { name: String, message: String },
    #[error(transparent)]
    DeviceCredential(#[from] DeviceCredentialError),
}

impl GatewayConfig {
    pub fn from_env() -> Result<Self, GatewayConfigError> {
        Self::from_map(&std::env::vars().collect())
    }

    pub fn from_map(values: &HashMap<String, String>) -> Result<Self, GatewayConfigError> {
        let credentials = credentials(values)?;
        let public_base_url = required(values, "SIMDOREI_MCP_PUBLIC_BASE_URL")?
            .parse::<Url>()
            .map_err(|error| invalid("SIMDOREI_MCP_PUBLIC_BASE_URL", error))?;
        let owner_token = required(values, "SIMDOREI_MCP_OWNER_TOKEN")?.to_owned();
        let oauth_database_path = PathBuf::from(optional(
            values,
            "SIMDOREI_MCP_OAUTH_DATABASE_PATH",
            "/data/oauth.sqlite3",
        ));
        let access = integer(values, "OAUTH_ACCESS_TOKEN_SECONDS", 3_600, 300, 86_400)?;
        let refresh = integer(
            values,
            "OAUTH_REFRESH_TOKEN_SECONDS",
            2_592_000,
            3_600,
            31_536_000,
        )?;
        let pending = integer(values, "OAUTH_PENDING_AUTHORIZATION_LIMIT", 100, 1, 1_000)?;
        let code_global = integer(
            values,
            "OAUTH_AUTHORIZATION_CODE_GLOBAL_LIMIT",
            1_024,
            1,
            100_000,
        )?;
        let code_client = integer(
            values,
            "OAUTH_AUTHORIZATION_CODE_PER_CLIENT_LIMIT",
            64,
            1,
            10_000,
        )?;
        let request_timeout = integer(values, "REQUEST_TIMEOUT_SECONDS", 3_630, 3_630, 7_200)?;
        let bind_address = optional(values, "SIMDOREI_MCP_BIND", "0.0.0.0:8030")
            .parse()
            .map_err(|error| invalid("SIMDOREI_MCP_BIND", error))?;
        let oauth = OAuthProviderConfig {
            public_base_url: public_base_url.clone(),
            owner_token: owner_token.clone(),
            access_token_seconds: access,
            refresh_token_seconds: refresh,
            pending_authorization_limit: usize_value(pending, "OAUTH_PENDING_AUTHORIZATION_LIMIT")?,
            authorization_code_limit: usize_value(
                code_global,
                "OAUTH_AUTHORIZATION_CODE_GLOBAL_LIMIT",
            )?,
            authorization_code_per_client_limit: usize_value(
                code_client,
                "OAUTH_AUTHORIZATION_CODE_PER_CLIENT_LIMIT",
            )?,
        };
        Ok(Self {
            credentials,
            oauth_database_path,
            oauth,
            store_limits: store_limits(values)?,
            request_timeout_seconds: u64_value(request_timeout, "REQUEST_TIMEOUT_SECONDS")?,
            bind_address,
        })
    }

    #[must_use]
    pub const fn bind_address(&self) -> SocketAddr {
        self.bind_address
    }

    #[must_use]
    pub fn configured_device_count(&self) -> usize {
        self.credentials.configured_count()
    }
}

fn credentials(
    values: &HashMap<String, String>,
) -> Result<DeviceCredentialRegistry, GatewayConfigError> {
    let registry = value(values, "SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON");
    let legacy_id = value(values, "SIMDOREI_MCP_DEVICE_ID");
    let legacy_token = value(values, "SIMDOREI_MCP_DEVICE_TOKEN");
    if legacy_id.is_empty() != legacy_token.is_empty() {
        return Err(GatewayConfigError::PartialLegacyCredential);
    }
    if !registry.is_empty() {
        let parsed = DeviceCredentialRegistry::from_json(registry)?;
        if !legacy_id.is_empty() && !parsed.matches_pair(legacy_id, legacy_token) {
            return Err(GatewayConfigError::LegacyCredentialMismatch);
        }
        return Ok(parsed);
    }
    if legacy_id.is_empty() {
        return Err(GatewayConfigError::Missing(
            "SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON or legacy device credentials",
        ));
    }
    DeviceCredentialRegistry::new(vec![DeviceCredential::new(legacy_id, legacy_token)?])
        .map_err(Into::into)
}

fn store_limits(values: &HashMap<String, String>) -> Result<OAuthStoreLimits, GatewayConfigError> {
    Ok(OAuthStoreLimits {
        max_clients: integer(values, "OAUTH_CLIENT_LIMIT", 500, 10, 10_000)?,
        max_token_families: integer(values, "OAUTH_TOKEN_FAMILY_GLOBAL_LIMIT", 256, 1, 100_000)?,
        max_token_families_per_client: integer(
            values,
            "OAUTH_TOKEN_FAMILY_PER_CLIENT_LIMIT",
            16,
            1,
            10_000,
        )?,
        max_refresh_history_global: integer(
            values,
            "OAUTH_REFRESH_HISTORY_GLOBAL_LIMIT",
            65_536,
            1,
            1_000_000,
        )?,
        max_refresh_history_per_family: integer(
            values,
            "OAUTH_REFRESH_HISTORY_PER_FAMILY_LIMIT",
            1_024,
            1,
            100_000,
        )?,
    })
}

fn integer(
    values: &HashMap<String, String>,
    suffix: &'static str,
    default: i64,
    min: i64,
    max: i64,
) -> Result<i64, GatewayConfigError> {
    let name = format!("{PREFIX}{suffix}");
    let raw = value(values, &name);
    let parsed = if raw.is_empty() {
        default
    } else {
        raw.parse().map_err(|error| invalid(&name, error))?
    };
    if (min..=max).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(invalid(&name, format!("must be between {min} and {max}")))
    }
}

fn required<'a>(
    values: &'a HashMap<String, String>,
    name: &'static str,
) -> Result<&'a str, GatewayConfigError> {
    let result = value(values, name);
    (!result.is_empty())
        .then_some(result)
        .ok_or(GatewayConfigError::Missing(name))
}

fn optional<'a>(values: &'a HashMap<String, String>, name: &str, default: &'a str) -> &'a str {
    let result = value(values, name);
    if result.is_empty() { default } else { result }
}

fn value<'a>(values: &'a HashMap<String, String>, name: &str) -> &'a str {
    values.get(name).map_or("", |value| value.trim())
}

fn invalid(name: &str, error: impl std::fmt::Display) -> GatewayConfigError {
    GatewayConfigError::Invalid {
        name: name.to_owned(),
        message: error.to_string(),
    }
}

fn usize_value(value: i64, name: &'static str) -> Result<usize, GatewayConfigError> {
    usize::try_from(value).map_err(|error| invalid(name, error))
}

fn u64_value(value: i64, name: &'static str) -> Result<u64, GatewayConfigError> {
    u64::try_from(value).map_err(|error| invalid(name, error))
}
