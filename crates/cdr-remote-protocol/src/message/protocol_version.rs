use std::fmt;

use cdr_core::ValidationError;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

pub const PROTOCOL_VERSION: u8 = 10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProtocolV10;

impl Serialize for ProtocolV10 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(PROTOCOL_VERSION)
    }
}

impl<'de> Deserialize<'de> for ProtocolV10 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = u8::deserialize(deserializer)?;
        if version == PROTOCOL_VERSION {
            Ok(Self)
        } else {
            Err(de::Error::custom(format!(
                "protocol version {version} is unsupported; expected {PROTOCOL_VERSION}"
            )))
        }
    }
}

impl fmt::Display for ProtocolV10 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("10")
    }
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid protocol JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("protocol version {received} is unsupported; expected {PROTOCOL_VERSION}")]
    UnsupportedVersion { received: u64 },
    #[error("protocol validation failed: {0}")]
    Validation(#[from] ValidationError),
}

pub(super) fn reject_unsupported_version(value: &serde_json::Value) -> Result<(), ProtocolError> {
    if let Some(received) = value
        .get("protocol_version")
        .and_then(serde_json::Value::as_u64)
        && received != u64::from(PROTOCOL_VERSION)
    {
        return Err(ProtocolError::UnsupportedVersion { received });
    }
    Ok(())
}
