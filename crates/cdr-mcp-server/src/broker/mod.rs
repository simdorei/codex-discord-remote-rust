mod connection;
mod device_selection;
mod dispatch;
mod error;
mod model;
mod operations;
mod selection;
mod state;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

pub use error::BrokerError;
pub use model::{
    BridgeAttachment, BridgeIdentity, DeviceSelection, DeviceSummary, ProjectRegistration,
    ProjectSelection,
};
use state::BrokerState;

#[derive(Clone)]
pub struct BridgeBroker {
    state: Arc<Mutex<BrokerState>>,
    selection: Arc<Mutex<()>>,
    request_timeout: Duration,
}

impl BridgeBroker {
    #[must_use]
    pub fn new(request_timeout: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(BrokerState::default())),
            selection: Arc::new(Mutex::new(())),
            request_timeout,
        }
    }
}
