use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::GatewayConfig;
use crate::bridge_http::bridge_router;
use crate::broker::BridgeBroker;
use crate::mcp_http::mcp_router;
use crate::oauth::{OAuthProvider, OAuthProviderError};
use crate::oauth_http::oauth_router;
use crate::oauth_store::{OAuthStore, OAuthStoreError};
use crate::tool_dispatcher::BrokerToolDispatcher;

#[derive(Debug, Error)]
pub enum GatewayServerError {
    #[error(transparent)]
    Store(#[from] OAuthStoreError),
    #[error(transparent)]
    OAuth(#[from] OAuthProviderError),
}

#[derive(Clone)]
struct HealthState {
    broker: BridgeBroker,
    configured_devices: usize,
}

pub fn gateway_router(
    config: GatewayConfig,
    cancellation: CancellationToken,
) -> Result<Router, GatewayServerError> {
    let configured_devices = config.credentials.configured_count();
    let store = Arc::new(OAuthStore::open(
        &config.oauth_database_path,
        config.store_limits,
    )?);
    let provider = OAuthProvider::new(store, config.oauth)?;
    let broker = BridgeBroker::new(Duration::from_secs(config.request_timeout_seconds));
    let dispatcher = Arc::new(BrokerToolDispatcher::new(
        broker.clone(),
        provider.resource_url().to_string(),
    ));
    let health = Router::new()
        .route("/healthz", get(health))
        .with_state(HealthState {
            broker: broker.clone(),
            configured_devices,
        });
    let mcp = mcp_router(&provider, dispatcher, cancellation)?;
    Ok(oauth_router(provider)
        .merge(bridge_router(config.credentials, broker))
        .merge(mcp)
        .merge(health))
}

async fn health(State(state): State<HealthState>) -> Json<Value> {
    let connected_devices = state.broker.list_devices().await.len();
    Json(json!({
        "ok": true,
        "service": "simdorei-local-project-mcp",
        "upstream_ready": connected_devices > 0,
        "configured_devices": state.configured_devices,
        "connected_devices": connected_devices,
    }))
}
