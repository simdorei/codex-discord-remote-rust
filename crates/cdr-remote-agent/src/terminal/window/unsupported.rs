use std::path::Path;

use cdr_remote_protocol::output::TerminalWindowEntry;
use cdr_remote_protocol::request::TerminalWindowShell;

use super::{
    OwnedTerminalWindow, TerminalWindowBackend, TerminalWindowCapture,
    TerminalWindowInteractionBackend, TerminalWindowObservation,
};
use crate::terminal::TerminalError;

pub struct UnsupportedWindowBackend;

impl TerminalWindowBackend for UnsupportedWindowBackend {
    fn open(
        &self,
        _id: &str,
        _shell: TerminalWindowShell,
        _cwd: &Path,
        _title: &str,
    ) -> Result<OwnedTerminalWindow, TerminalError> {
        Err(unsupported())
    }
    fn inspect(
        &self,
        _window: &OwnedTerminalWindow,
    ) -> Result<Option<TerminalWindowEntry>, TerminalError> {
        Err(unsupported())
    }
    fn close(&self, _window: &OwnedTerminalWindow) -> Result<(), TerminalError> {
        Err(unsupported())
    }
}

impl TerminalWindowInteractionBackend for UnsupportedWindowBackend {
    fn capture(
        &self,
        _window: &OwnedTerminalWindow,
    ) -> Result<TerminalWindowCapture, TerminalError> {
        Err(unsupported())
    }
    fn activate(&self, _window: &OwnedTerminalWindow) -> Result<bool, TerminalError> {
        Err(unsupported())
    }
    fn type_text(&self, _window: &OwnedTerminalWindow, _text: &str) -> Result<bool, TerminalError> {
        Err(unsupported())
    }
    fn press_keys(
        &self,
        _window: &OwnedTerminalWindow,
        _keys: &[String],
    ) -> Result<(bool, Vec<String>), TerminalError> {
        Err(unsupported())
    }
    fn interrupt(&self, _window: &OwnedTerminalWindow) -> Result<(), TerminalError> {
        Err(unsupported())
    }
    fn matches_observation(
        &self,
        _window: &OwnedTerminalWindow,
        _observation: &TerminalWindowObservation,
    ) -> bool {
        false
    }
}

fn unsupported() -> TerminalError {
    TerminalError::Window("visible terminal windows are supported only on Windows".into())
}
