use std::collections::HashMap;
use std::fmt;
use std::hash::BuildHasher;

use thiserror::Error;
use url::Url;

#[derive(Clone, Eq, PartialEq)]
pub struct SecretToken(String);

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken([REDACTED])")
    }
}

impl SecretToken {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteMcpConfig {
    pub bridge_url: Url,
    pub device_id: String,
    pub device_token: SecretToken,
    pub binding_ttl_seconds: u64,
    pub binding_ack_timeout_seconds: u64,
    pub reconnect_delay_seconds: u64,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ConfigError {
    #[error("{name} is required when remote MCP is enabled.")]
    Missing { name: &'static str },
    #[error("CODEX_REMOTE_MCP_BRIDGE_URL must use wss:// (ws:// is allowed only for localhost).")]
    InvalidBridgeUrl,
    #[error("{name} must be an integer between {minimum} and {maximum}.")]
    InvalidInteger {
        name: &'static str,
        minimum: u64,
        maximum: u64,
    },
}

pub fn load_remote_mcp_config<S: BuildHasher>(
    values: &HashMap<String, String, S>,
) -> Result<Option<RemoteMcpConfig>, ConfigError> {
    let enabled = values
        .get("CODEX_REMOTE_MCP_ENABLED")
        .map_or("", String::as_str)
        .trim()
        .to_ascii_lowercase();
    if !matches!(enabled.as_str(), "1" | "true" | "yes" | "on") {
        return Ok(None);
    }
    let raw_url = required(values, "CODEX_REMOTE_MCP_BRIDGE_URL")?;
    let bridge_url = Url::parse(raw_url).map_err(|_| ConfigError::InvalidBridgeUrl)?;
    if !secure_bridge_url(&bridge_url) {
        return Err(ConfigError::InvalidBridgeUrl);
    }
    Ok(Some(RemoteMcpConfig {
        bridge_url,
        device_id: required(values, "CODEX_REMOTE_MCP_DEVICE_ID")?.to_owned(),
        device_token: SecretToken(required(values, "CODEX_REMOTE_MCP_DEVICE_TOKEN")?.to_owned()),
        binding_ttl_seconds: bounded_integer(
            values,
            "CODEX_REMOTE_MCP_BINDING_TTL_SECONDS",
            1_800,
            60,
            86_400,
        )?,
        binding_ack_timeout_seconds: 10,
        reconnect_delay_seconds: 2,
    }))
}

fn required<'a, S: BuildHasher>(
    values: &'a HashMap<String, String, S>,
    name: &'static str,
) -> Result<&'a str, ConfigError> {
    values
        .get(name)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::Missing { name })
}

fn secure_bridge_url(url: &Url) -> bool {
    if url.host_str().is_none()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    if url.scheme() == "wss" {
        return true;
    }
    url.scheme() == "ws"
        && url.host_str().is_some_and(|host| {
            matches!(
                host.to_ascii_lowercase().as_str(),
                "127.0.0.1" | "::1" | "[::1]" | "localhost"
            )
        })
}

fn bounded_integer<S: BuildHasher>(
    values: &HashMap<String, String, S>,
    name: &'static str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, ConfigError> {
    let Some(raw) = values.get(name).map(String::as_str).map(str::trim) else {
        return Ok(default);
    };
    if raw.is_empty() {
        return Ok(default);
    }
    raw.parse::<u64>()
        .ok()
        .filter(|value| (minimum..=maximum).contains(value))
        .ok_or(ConfigError::InvalidInteger {
            name,
            minimum,
            maximum,
        })
}
