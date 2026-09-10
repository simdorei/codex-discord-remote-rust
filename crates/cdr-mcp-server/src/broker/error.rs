use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BrokerError {
    #[error("selected local bridge is disconnected")]
    BridgeUnavailable,
    #[error("project scope is unavailable or expired")]
    ProjectUnavailable,
    #[error("project scope is owned by another device")]
    ProjectOwnedByAnotherDevice,
    #[error("ChatGPT session has no active project selection")]
    ActiveSelectionMissing,
    #[error("ChatGPT session belongs to a different OAuth principal")]
    PrincipalMismatch,
    #[error("local bridge response timed out")]
    ResponseTimeout,
    #[error("local bridge request was cancelled")]
    Cancelled,
    #[error("local bridge returned no result")]
    MissingResult,
    #[error("local bridge returned the wrong result type")]
    WrongResultType,
    #[error("local bridge operation failed: {0}")]
    RemoteOperation(String),
    #[error("invalid local bridge request: {0}")]
    InvalidRequest(String),
}
