use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

pub trait EnvLookup {
    fn get_env(&self, key: &str) -> Option<&str>;
}

impl<S: BuildHasher> EnvLookup for HashMap<String, String, S> {
    fn get_env(&self, key: &str) -> Option<&str> {
        self.get(key).map(String::as_str)
    }
}

impl EnvLookup for BTreeMap<String, String> {
    fn get_env(&self, key: &str) -> Option<&str> {
        self.get(key).map(String::as_str)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteMcpOwnership {
    #[default]
    Embedded,
    External,
}

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
    pub ownership: RemoteMcpOwnership,
    pub bridge_url: Url,
    pub device_id: String,
    pub device_token: SecretToken,
    pub binding_ttl_seconds: u64,
    pub binding_ack_timeout_seconds: u64,
    pub reconnect_delay_seconds: u64,
    pub data_dir_override: Option<PathBuf>,
    pub status_file_override: Option<PathBuf>,
    pub stop_file_override: Option<PathBuf>,
}

impl RemoteMcpConfig {
    #[must_use]
    pub fn bridge_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(
            format!(
                "v1|url={}|device_id={}|ttl={}",
                self.bridge_url.as_str().trim(),
                self.device_id.trim(),
                self.binding_ttl_seconds
            )
            .as_bytes(),
        );
        hex::encode(hasher.finalize())
    }

    #[must_use]
    pub fn resolve_data_dir(&self, install_root: &Path) -> PathBuf {
        match &self.data_dir_override {
            Some(path) if path.is_absolute() => path.clone(),
            Some(path) => install_root.join(path),
            None => install_root.join("bot_data"),
        }
    }

    #[must_use]
    pub fn resolve_status_file_path(&self, install_root: &Path) -> PathBuf {
        match &self.status_file_override {
            Some(path) if path.is_absolute() => path.clone(),
            Some(path) => install_root.join(path),
            None => self
                .resolve_data_dir(install_root)
                .join("remote_mcp_status.json"),
        }
    }

    #[must_use]
    pub fn resolve_stop_file_path(&self, install_root: &Path) -> PathBuf {
        match &self.stop_file_override {
            Some(path) if path.is_absolute() => path.clone(),
            Some(path) => install_root.join(path),
            None => self.resolve_data_dir(install_root).join("remote_mcp.stop"),
        }
    }

    #[must_use]
    pub fn resolve_instance_lock_path(&self, install_root: &Path) -> PathBuf {
        self.resolve_data_dir(install_root)
            .join("remote_mcp.instance.lock")
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ConfigError {
    #[error("{name} is required when remote MCP is enabled.")]
    Missing { name: &'static str },
    #[error("CODEX_REMOTE_MCP_BRIDGE_URL must use wss:// (ws:// is allowed only for localhost).")]
    InvalidBridgeUrl,
    #[error("CODEX_REMOTE_MCP_OWNERSHIP must be embedded or external.")]
    InvalidOwnershipMode,
    #[error("{name} must be an integer between {minimum} and {maximum}.")]
    InvalidInteger {
        name: &'static str,
        minimum: u64,
        maximum: u64,
    },
}

pub fn load_remote_mcp_config<M: EnvLookup>(
    values: &M,
) -> Result<Option<RemoteMcpConfig>, ConfigError> {
    let ownership = parse_ownership_mode(values)?;
    let enabled = values
        .get_env("CODEX_REMOTE_MCP_ENABLED")
        .unwrap_or("")
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
        ownership,
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
        reconnect_delay_seconds: bounded_integer(
            values,
            "CODEX_REMOTE_MCP_RECONNECT_SECONDS",
            2,
            0,
            300,
        )?,
        data_dir_override: optional_path(values, &["CODEX_MCP_DATA_DIR", "CODEX_BOT_DATA_DIR"]),
        status_file_override: optional_path(values, &["CODEX_REMOTE_MCP_STATUS_FILE"]),
        stop_file_override: optional_path(values, &["CODEX_REMOTE_MCP_STOP_FILE"]),
    }))
}

fn optional_path<M: EnvLookup>(values: &M, keys: &[&str]) -> Option<PathBuf> {
    for key in keys {
        if let Some(raw) = values.get_env(key).map(str::trim)
            && !raw.is_empty()
        {
            return Some(PathBuf::from(raw));
        }
    }
    None
}

pub fn parse_ownership_mode<M: EnvLookup>(values: &M) -> Result<RemoteMcpOwnership, ConfigError> {
    let Some(raw) = values.get_env("CODEX_REMOTE_MCP_OWNERSHIP").map(str::trim) else {
        return Ok(RemoteMcpOwnership::Embedded);
    };
    if raw.is_empty() {
        return Ok(RemoteMcpOwnership::Embedded);
    }
    match raw.to_ascii_lowercase().as_str() {
        "embedded" => Ok(RemoteMcpOwnership::Embedded),
        "external" => Ok(RemoteMcpOwnership::External),
        _ => Err(ConfigError::InvalidOwnershipMode),
    }
}

fn required<'a, M: EnvLookup>(values: &'a M, name: &'static str) -> Result<&'a str, ConfigError> {
    values
        .get_env(name)
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

fn bounded_integer<M: EnvLookup>(
    values: &M,
    name: &'static str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, ConfigError> {
    let Some(raw) = values.get_env(name).map(str::trim) else {
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
