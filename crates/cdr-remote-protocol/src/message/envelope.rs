use cdr_core::{Validate, ValidationResult, length};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::protocol_version::reject_unsupported_version;
use super::{BridgeResult, GatewayCommand, ProtocolError, ProtocolV10};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BridgeInboundMessage {
    Hello {
        protocol_version: ProtocolV10,
        device_id: String,
    },
    ProjectUpsert {
        project_scope: String,
        binding_id: String,
        thread_id: String,
        project_name: String,
        expires_at: DateTime<Utc>,
    },
    #[serde(untagged)]
    Result(BridgeResult),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayInboundMessage {
    HelloAck {
        #[serde(default)]
        protocol_version: ProtocolV10,
    },
    ProjectAck {
        project_scope: String,
        binding_id: String,
    },
    #[serde(untagged)]
    Command(GatewayCommand),
}

pub fn parse_bridge_message(raw: &str) -> Result<BridgeInboundMessage, ProtocolError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    reject_unsupported_version(&value)?;
    let message: BridgeInboundMessage = serde_json::from_value(value)?;
    message.validate()?;
    Ok(message)
}

pub fn parse_gateway_message(raw: &str) -> Result<GatewayInboundMessage, ProtocolError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    reject_unsupported_version(&value)?;
    let message: GatewayInboundMessage = serde_json::from_value(value)?;
    message.validate()?;
    Ok(message)
}

impl Validate for BridgeInboundMessage {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::Hello { .. } => Ok(()),
            Self::ProjectUpsert {
                project_scope,
                binding_id,
                project_name,
                ..
            } => {
                length("project_scope", project_scope, 12, 200)?;
                length("binding_id", binding_id, 16, 64)?;
                length("project_name", project_name, 1, 200)
            }
            Self::Result(result) => result.validate(),
        }
    }
}

impl Validate for GatewayInboundMessage {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::HelloAck { .. } => Ok(()),
            Self::ProjectAck { binding_id, .. } => length("binding_id", binding_id, 16, 64),
            Self::Command(command) => command.validate(),
        }
    }
}
