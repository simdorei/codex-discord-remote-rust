mod credentials;
mod socket;

use axum::Router;
use axum::routing::get;

pub use credentials::{DeviceCredential, DeviceCredentialError, DeviceCredentialRegistry};
use socket::BridgeHttpState;

use crate::broker::BridgeBroker;

pub fn bridge_router(credentials: DeviceCredentialRegistry, broker: BridgeBroker) -> Router {
    Router::new()
        .route("/bridge", get(socket::upgrade))
        .with_state(BridgeHttpState {
            credentials,
            broker,
        })
}
