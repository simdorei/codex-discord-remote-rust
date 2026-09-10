use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartProject {
    pub thread_id: String,
    pub root: PathBuf,
    pub expires_at: DateTime<Utc>,
}

pub trait HandoffProtector: Send + Sync {
    fn protect(&self, payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError>;
    fn unprotect(&self, payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError>;
}

pub struct SystemProtector;

#[derive(Debug, Error)]
pub enum RestartHandoffError {
    #[error("restart handoff I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("restart handoff protection failed: {0}")]
    Protection(String),
    #[error("restart handoff exceeds the size limit")]
    TooLarge,
    #[error("restart handoff is malformed")]
    Malformed,
    #[error("unsupported restart handoff format")]
    UnsupportedFormat,
    #[error("restart handoff protocol mismatch")]
    ProtocolMismatch,
    #[error("restart handoff gateway mismatch")]
    GatewayMismatch,
    #[error("restart handoff expired")]
    Expired,
    #[error("restart handoff has no projects")]
    NoProjects,
    #[error("restart project binding expired")]
    ProjectExpired,
    #[error("restart project root is unavailable")]
    ProjectRootUnavailable,
    #[error("restart handoff is only supported on Windows")]
    UnsupportedPlatform,
    #[error("the local restart handoff storage directory is unavailable")]
    LocalStateUnavailable,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    pub format_version: u8,
    pub protocol_version: u8,
    pub gateway_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub resume_until: DateTime<Utc>,
    pub projects: Vec<RestartProject>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Envelope {
    pub format_version: u8,
    pub ciphertext: String,
}
