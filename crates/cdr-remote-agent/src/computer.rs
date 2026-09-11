mod controller;
mod session;
#[cfg(windows)]
mod windows;

use cdr_remote_protocol::output::ComputerWindowEntry;
use cdr_remote_protocol::request::{ComputerApp, ComputerMouseButton};
use thiserror::Error;

pub use controller::ComputerController;
pub(crate) use session::SessionComputer;
#[cfg(windows)]
pub use windows::new_windows_controller;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerAccessMode {
    Project,
    Device,
}

pub fn new_computer_controller(
    mode: ComputerAccessMode,
) -> Result<ComputerController, ComputerError> {
    #[cfg(windows)]
    {
        Ok(new_windows_controller(mode))
    }
    #[cfg(not(windows))]
    {
        let _ = mode;
        Err(ComputerError::Platform(
            "Computer control is currently available only on Windows.".into(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerIdentity {
    pub window_id: u64,
    pub process_id: u32,
    pub process_path: String,
    pub title_digest: String,
    pub left: i64,
    pub top: i64,
    pub width: u64,
    pub height: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerCapture {
    pub window: ComputerWindowEntry,
    pub identity: ComputerIdentity,
    pub png: Vec<u8>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ComputerError {
    #[error("computer control is stopped until the project is bound again")]
    Stopped,
    #[error("a fresh screenshot is required")]
    FreshScreenshot,
    #[error("the screenshot belongs to a different window")]
    DifferentWindow,
    #[error("the requested coordinates are outside the screenshot")]
    OutsideScreenshot,
    #[error("computer platform operation failed: {0}")]
    Platform(String),
    #[error("computer controller state is unavailable")]
    State,
}

pub trait ComputerPlatform: Send + Sync {
    fn list_windows(&self) -> Result<Vec<ComputerWindowEntry>, ComputerError>;
    fn screenshot(&self, window_id: u64) -> Result<ComputerCapture, ComputerError>;
    fn activate(&self, window_id: u64) -> Result<ComputerWindowEntry, ComputerError>;
    fn launch(&self, app: ComputerApp) -> Result<(), ComputerError>;
    fn click(
        &self,
        identity: &ComputerIdentity,
        x: u32,
        y: u32,
        button: ComputerMouseButton,
        count: u8,
    ) -> Result<(), ComputerError>;
    fn drag(
        &self,
        identity: &ComputerIdentity,
        start: (u32, u32),
        end: (u32, u32),
    ) -> Result<(), ComputerError>;
    fn scroll(
        &self,
        identity: &ComputerIdentity,
        point: (u32, u32),
        delta: (i32, i32),
    ) -> Result<(), ComputerError>;
    fn type_text(&self, identity: &ComputerIdentity, text: &str) -> Result<(), ComputerError>;
    fn press_keys(&self, identity: &ComputerIdentity, keys: &[String])
    -> Result<(), ComputerError>;
    fn close(&self, identity: &ComputerIdentity) -> Result<(), ComputerError>;
    fn set_clipboard(&self, identity: &ComputerIdentity, text: &str) -> Result<(), ComputerError>;
    fn stop(&self) -> Result<(), ComputerError>;
}
