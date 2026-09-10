use thiserror::Error;

#[derive(Debug, Error)]
pub enum NativeError {
    #[error("{operation} failed: {source}")]
    Api {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} contains a NUL character")]
    InteriorNul(&'static str),
    #[error("timed out stopping the owned Windows process tree")]
    StopTimeout,
    #[error("terminal window did not open before the deadline")]
    WindowOpenTimeout,
    #[error("terminal window is no longer available")]
    WindowMissing,
    #[error("terminal window identity changed")]
    IdentityChanged,
    #[error("terminal window has invalid bounds")]
    InvalidBounds,
    #[error("terminal window is too large to capture")]
    CaptureTooLarge,
    #[error("Windows returned incomplete screenshot data")]
    IncompleteCapture,
    #[error("{0}")]
    InvalidInput(String),
    #[error("unexpected Windows wait result: {0}")]
    UnexpectedWait(u32),
    #[error("another Codex Discord Remote Rust runtime is already running")]
    AlreadyRunning,
    #[error("PNG encoding failed: {0}")]
    Png(#[from] png::EncodingError),
}

pub(crate) fn api_error(operation: &'static str) -> NativeError {
    NativeError::Api {
        operation,
        source: std::io::Error::last_os_error(),
    }
}
