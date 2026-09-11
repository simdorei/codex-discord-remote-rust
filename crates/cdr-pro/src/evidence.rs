mod browser;
pub(crate) mod common;
mod connector;
pub(crate) mod connector_transcript;

use thiserror::Error;

pub use browser::{
    canonical_browser_inner_probe_code, canonical_browser_probe_code, require_browser_available,
    require_browser_available_with_transcript,
};
pub use connector::require_connector_verified;
pub use connector_transcript::{
    CONNECTOR_NAME, CONNECTOR_PATH, CONNECTOR_PROBE_SHA256, CONNECTOR_PROTOCOL,
    canonical_connector_inner_probe_code, canonical_connector_probe_code,
    canonical_connector_retry_probe_code,
};

pub const BROWSER_EVIDENCE_PROTOCOL: &str = "ask-chatgpt-pro-browser-evidence-v2";
pub const BROWSER_PROBE_SHA256: &str =
    "346dfe09965e91f6b4339e6c1fcfb14a76c204ef3972779780847d2f7b358853";

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("pro_chrome_unavailable")]
    ChromeUnavailable { internal_detail: String },
    #[error("pro_connector_unavailable")]
    ConnectorUnavailable { internal_detail: String },
    #[error("evidence path cannot be represented as a file URI: {0}")]
    InvalidFilePath(String),
}

pub type Result<T> = std::result::Result<T, EvidenceError>;

fn chrome_unavailable(detail: impl Into<String>) -> EvidenceError {
    EvidenceError::ChromeUnavailable {
        internal_detail: detail.into(),
    }
}

fn connector_unavailable(detail: impl Into<String>) -> EvidenceError {
    EvidenceError::ConnectorUnavailable {
        internal_detail: detail.into(),
    }
}
