use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

const MAX_CREDENTIALS: usize = 32;
const MAX_JSON_BYTES: usize = 24 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DeviceCredentialError {
    #[error("device credential registry must contain between 1 and 32 devices")]
    InvalidCount,
    #[error("device ID must use 1 to 128 portable ASCII characters")]
    InvalidDeviceId,
    #[error("device token must contain 32 to 512 printable ASCII characters without whitespace")]
    InvalidToken,
    #[error("device IDs and tokens must be unique")]
    Duplicate,
    #[error("device credential JSON exceeds 24 KiB")]
    JsonTooLarge,
    #[error("invalid device credential JSON: {0}")]
    Json(String),
}

pub struct DeviceCredential {
    device_id: String,
    token: String,
}

impl DeviceCredential {
    pub fn new(device_id: &str, token: &str) -> Result<Self, DeviceCredentialError> {
        if !device_pattern().is_match(device_id) {
            return Err(DeviceCredentialError::InvalidDeviceId);
        }
        if !(32..=512).contains(&token.len())
            || !token.is_ascii()
            || token.bytes().any(|byte| !(33..=126).contains(&byte))
        {
            return Err(DeviceCredentialError::InvalidToken);
        }
        Ok(Self {
            device_id: device_id.to_owned(),
            token: token.to_owned(),
        })
    }
}

#[derive(Clone)]
pub struct DeviceCredentialRegistry {
    credentials: Vec<StoredCredential>,
}

#[derive(Clone)]
struct StoredCredential {
    device_id: String,
    token_digest: [u8; 32],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryJson {
    version: u8,
    devices: Vec<CredentialJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialJson {
    device_id: String,
    token: String,
}

impl DeviceCredentialRegistry {
    pub fn new(devices: Vec<DeviceCredential>) -> Result<Self, DeviceCredentialError> {
        if devices.is_empty() || devices.len() > MAX_CREDENTIALS {
            return Err(DeviceCredentialError::InvalidCount);
        }
        let mut ids = HashSet::new();
        let mut digests = HashSet::new();
        let mut credentials = Vec::with_capacity(devices.len());
        for device in devices {
            let digest = digest(device.token.as_bytes());
            if !ids.insert(device.device_id.clone()) || !digests.insert(digest) {
                return Err(DeviceCredentialError::Duplicate);
            }
            credentials.push(StoredCredential {
                device_id: device.device_id,
                token_digest: digest,
            });
        }
        Ok(Self { credentials })
    }

    pub fn from_json(value: &str) -> Result<Self, DeviceCredentialError> {
        if value.len() > MAX_JSON_BYTES {
            return Err(DeviceCredentialError::JsonTooLarge);
        }
        let parsed: RegistryJson = serde_json::from_str(value)
            .map_err(|error| DeviceCredentialError::Json(error.to_string()))?;
        if parsed.version != 1 {
            return Err(DeviceCredentialError::Json(
                "device credential version must be 1".into(),
            ));
        }
        let devices = parsed
            .devices
            .into_iter()
            .map(|value| DeviceCredential::new(&value.device_id, &value.token))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(devices)
    }

    pub(crate) fn authenticate(&self, authorization: &str) -> Option<String> {
        let token = authorization.strip_prefix("Bearer ")?;
        let candidate = digest(token.as_bytes());
        let mut matched_device = None;
        let mut match_count = 0_u8;
        for credential in &self.credentials {
            let is_match = credential.token_digest.ct_eq(&candidate).unwrap_u8();
            match_count = match_count.saturating_add(is_match);
            if is_match == 1 {
                matched_device = Some(credential.device_id.clone());
            }
        }
        (match_count == 1).then_some(matched_device).flatten()
    }

    #[must_use]
    pub fn configured_count(&self) -> usize {
        self.credentials.len()
    }

    pub(crate) fn matches_pair(&self, device_id: &str, token: &str) -> bool {
        self.authenticate(&format!("Bearer {token}"))
            .is_some_and(|matched| same_id(&matched, device_id))
    }
}

fn digest(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn device_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$").expect("valid device ID regex")
    })
}

fn same_id(left: &str, right: &str) -> bool {
    digest(left.as_bytes())
        .ct_eq(&digest(right.as_bytes()))
        .into()
}
