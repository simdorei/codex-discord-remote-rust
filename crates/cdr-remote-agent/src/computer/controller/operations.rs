use cdr_remote_protocol::output::{ComputerActionName, ComputerOutput};
use cdr_remote_protocol::request::{ComputerApp, ComputerRequest};

use super::ComputerController;
use crate::computer::ComputerError;

impl ComputerController {
    pub(super) fn execute_running(
        &self,
        request: &ComputerRequest,
    ) -> Result<ComputerOutput, ComputerError> {
        match request {
            ComputerRequest::ComputerListWindows => Ok(ComputerOutput::ComputerWindows {
                windows: self.platform.list_windows()?,
            }),
            ComputerRequest::ComputerActivate { window_id } => {
                let window = self.platform.activate(*window_id)?;
                Ok(action(
                    ComputerActionName::Activate,
                    Some(window.window_id),
                    "Window activated.",
                ))
            }
            ComputerRequest::ComputerLaunch { app } => {
                self.platform.launch(*app)?;
                Ok(action(
                    ComputerActionName::Launch,
                    None,
                    match app {
                        ComputerApp::Chrome => "chrome launch requested.",
                        ComputerApp::Notepad => "notepad launch requested.",
                    },
                ))
            }
            ComputerRequest::ComputerScreenshot { window_id } => self.screenshot(*window_id),
            ComputerRequest::ComputerStop => {
                unreachable!("stop is handled before the operation lock")
            }
            _ => self.execute_observed(request),
        }
    }
}

pub(super) fn action(
    action: ComputerActionName,
    window_id: Option<u64>,
    message: &str,
) -> ComputerOutput {
    ComputerOutput::ComputerAction {
        action,
        window_id,
        message: message.into(),
    }
}
