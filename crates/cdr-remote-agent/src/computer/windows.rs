mod device;
mod model;
mod policy;
mod project;

use std::sync::Arc;
use std::time::Duration;

use device::WindowsDevicePlatform;
use project::WindowsProjectPlatform;

use super::{ComputerAccessMode, ComputerController, ComputerPlatform};

const OBSERVATION_TTL: Duration = Duration::from_secs(30);

#[must_use]
pub fn new_windows_controller(mode: ComputerAccessMode) -> ComputerController {
    let platform: Arc<dyn ComputerPlatform> = match mode {
        ComputerAccessMode::Project => Arc::new(WindowsProjectPlatform::new()),
        ComputerAccessMode::Device => Arc::new(WindowsDevicePlatform::new()),
    };
    ComputerController::with_platform_and_ttl(platform, OBSERVATION_TTL)
}
