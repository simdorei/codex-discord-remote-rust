use std::path::Path;

use cdr_remote_protocol::output::{TerminalWindowEntry, TerminalWindowRect};
use cdr_remote_protocol::request::TerminalWindowShell;

use crate::terminal::TerminalError;

#[derive(Clone)]
pub struct OwnedTerminalWindow {
    pub entry: TerminalWindowEntry,
    pub backend_id: u64,
    pub window_process_id: Option<u32>,
}

pub struct TerminalWindowCapture {
    pub rect: TerminalWindowRect,
    pub png: Vec<u8>,
}

#[derive(Clone)]
pub struct TerminalWindowObservation {
    pub observation_id: cdr_remote_protocol::identifiers::TerminalWindowObservationId,
    pub terminal_window_id: cdr_remote_protocol::identifiers::TerminalWindowId,
    pub identity_digest: String,
    pub window_process_id: u32,
    pub rect: TerminalWindowRect,
}

pub trait TerminalWindowBackend: Send + Sync {
    fn open(
        &self,
        terminal_window_id: &str,
        shell: TerminalWindowShell,
        cwd: &Path,
        title: &str,
    ) -> Result<OwnedTerminalWindow, TerminalError>;
    fn inspect(
        &self,
        window: &OwnedTerminalWindow,
    ) -> Result<Option<TerminalWindowEntry>, TerminalError>;
    fn close(&self, window: &OwnedTerminalWindow) -> Result<(), TerminalError>;
}

pub trait TerminalWindowInteractionBackend: Send + Sync {
    fn capture(&self, window: &OwnedTerminalWindow)
    -> Result<TerminalWindowCapture, TerminalError>;
    fn activate(&self, window: &OwnedTerminalWindow) -> Result<bool, TerminalError>;
    fn type_text(&self, window: &OwnedTerminalWindow, text: &str) -> Result<bool, TerminalError>;
    fn press_keys(
        &self,
        window: &OwnedTerminalWindow,
        keys: &[String],
    ) -> Result<(bool, Vec<String>), TerminalError>;
    fn interrupt(&self, window: &OwnedTerminalWindow) -> Result<(), TerminalError>;
    fn matches_observation(
        &self,
        window: &OwnedTerminalWindow,
        observation: &TerminalWindowObservation,
    ) -> bool;
}
