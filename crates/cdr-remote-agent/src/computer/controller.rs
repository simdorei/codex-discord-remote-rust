mod actions;
mod operations;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cdr_remote_protocol::output::{ComputerOutput, ComputerScreenshotMediaType};
use cdr_remote_protocol::request::ComputerRequest;
use uuid::Uuid;

use super::{ComputerError, ComputerIdentity, ComputerPlatform};

struct Observation {
    identity: ComputerIdentity,
    expires_at: Instant,
}

pub struct ComputerController {
    platform: Arc<dyn ComputerPlatform>,
    ttl: Duration,
    observations: Mutex<HashMap<String, Observation>>,
    operation: Mutex<()>,
    stopped: AtomicBool,
    platform_stopped: AtomicBool,
}

impl ComputerController {
    pub fn with_platform_and_ttl(platform: Arc<dyn ComputerPlatform>, ttl: Duration) -> Self {
        Self {
            platform,
            ttl,
            observations: Mutex::new(HashMap::new()),
            operation: Mutex::new(()),
            stopped: AtomicBool::new(false),
            platform_stopped: AtomicBool::new(false),
        }
    }

    pub fn execute(&self, request: &ComputerRequest) -> Result<ComputerOutput, ComputerError> {
        if matches!(request, ComputerRequest::ComputerStop) {
            return self.stop();
        }
        self.ensure_running()?;
        let _operation = self.operation.lock().map_err(|_| ComputerError::State)?;
        self.ensure_running()?;
        let output = self.execute_running(request)?;
        self.ensure_running()?;
        Ok(output)
    }

    fn screenshot(&self, window_id: u64) -> Result<ComputerOutput, ComputerError> {
        let capture = self.platform.screenshot(window_id)?;
        let observation_id = Uuid::new_v4().simple().to_string();
        let mut observations = self.observations.lock().map_err(|_| ComputerError::State)?;
        observations.clear();
        observations.insert(
            observation_id.clone(),
            Observation {
                identity: capture.identity,
                expires_at: Instant::now() + self.ttl,
            },
        );
        Ok(ComputerOutput::ComputerScreenshot {
            observation_id,
            window: capture.window,
            media_type: ComputerScreenshotMediaType::Png,
            data_base64: STANDARD.encode(capture.png),
        })
    }

    fn consume(
        &self,
        observation_id: &str,
        window_id: u64,
        points: &[(u32, u32)],
    ) -> Result<ComputerIdentity, ComputerError> {
        let observation = self
            .observations
            .lock()
            .map_err(|_| ComputerError::State)?
            .remove(observation_id)
            .ok_or(ComputerError::FreshScreenshot)?;
        if Instant::now() >= observation.expires_at {
            return Err(ComputerError::FreshScreenshot);
        }
        if observation.identity.window_id != window_id {
            return Err(ComputerError::DifferentWindow);
        }
        if points.iter().any(|(x, y)| {
            u64::from(*x) >= observation.identity.width
                || u64::from(*y) >= observation.identity.height
        }) {
            return Err(ComputerError::OutsideScreenshot);
        }
        Ok(observation.identity)
    }

    fn stop(&self) -> Result<ComputerOutput, ComputerError> {
        self.stopped.store(true, Ordering::Release);
        let _operation = self.operation.lock().map_err(|_| ComputerError::State)?;
        self.observations
            .lock()
            .map_err(|_| ComputerError::State)?
            .clear();
        if !self.platform_stopped.load(Ordering::Acquire) {
            self.platform.stop()?;
            self.platform_stopped.store(true, Ordering::Release);
        }
        Ok(ComputerOutput::ComputerStop {
            stopped: true,
            message: "Computer control stopped until this project is bound again.".into(),
        })
    }

    fn ensure_running(&self) -> Result<(), ComputerError> {
        if self.stopped.load(Ordering::Acquire) {
            Err(ComputerError::Stopped)
        } else {
            Ok(())
        }
    }
}
