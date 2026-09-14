use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::RequestId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcErrorPayload {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Error)]
pub enum AppServerError {
    #[error("idle subscription release: {message}")]
    IdleRelease { message: String },
    #[error("dead app-server work could not be durably fenced: {message}")]
    DeadGenerationFence { message: String },
    #[error("could not start Codex app-server at {executable}: {source}")]
    Spawn {
        executable: String,
        #[source]
        source: std::io::Error,
    },
    #[cfg(windows)]
    #[error("Windows app-server process management failed: {0}")]
    WindowsProcess(#[from] cdr_windows_native::NativeError),
    #[error("app-server inherited environment {field} is not valid Unicode")]
    EnvironmentEncoding { field: &'static str },
    #[error("app-server I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("app-server JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("app-server request {method} timed out after {timeout_ms} ms")]
    Timeout { method: String, timeout_ms: u128 },
    #[error("app-server request channel closed for {method}")]
    ResponseChannelClosed { method: String },
    #[error("app-server returned error {code} for {method}: {message}")]
    Remote {
        method: String,
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("app-server transport closed while awaiting {method}: {reason}")]
    TransportClosed { method: String, reason: String },
    #[error("app-server transport is closed")]
    Closed,
    #[error("app-server process did not expose {stream}")]
    MissingPipe { stream: &'static str },
    #[error("invalid app-server reply: {message}")]
    InvalidReply { message: String },
    #[error("app-server generation mismatch: expected {expected}, current {actual}")]
    GenerationMismatch { expected: u64, actual: u64 },
    #[error("app-server generation {generation} is quarantined after an ambiguous timeout")]
    GenerationQuarantined { generation: u64 },
    #[error("app-server request {id:?} is stale or no longer pending")]
    StaleServerRequest { id: RequestId },
    #[error("app-server request {id:?} already has a response in flight")]
    ServerRequestResponseInFlight { id: RequestId },
    #[error("app-server request {id:?} response delivery is indeterminate")]
    ServerRequestResponseIndeterminate { id: RequestId },
    #[error("app-server startup failed: {primary}; cleanup also failed: {cleanup}")]
    StartupCleanup {
        #[source]
        primary: Box<AppServerError>,
        cleanup: Box<AppServerError>,
    },
    #[error("app-server startup cleanup task failed: {message}")]
    StartupCleanupTask { message: String },
    #[error("resident app-server replacement state invalid: {message}")]
    ReplacementState { message: String },
}
