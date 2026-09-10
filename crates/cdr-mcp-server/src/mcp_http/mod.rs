mod auth;
mod handler;
mod tools;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn_with_state;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::oauth::{OAuthProvider, OAuthProviderError};

#[derive(Clone, Debug)]
pub struct ToolCallContext {
    pub session: String,
    pub subject: String,
    pub request_id: String,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DispatchOutput {
    Structured(Value),
    Image { data: String, mime_type: String },
}

pub trait ToolDispatcher: Send + Sync + 'static {
    fn dispatch(
        &self,
        context: ToolCallContext,
        tool: String,
        arguments: Map<String, Value>,
    ) -> Pin<Box<dyn Future<Output = Result<DispatchOutput, String>> + Send>>;
}

pub fn mcp_router(
    provider: &OAuthProvider,
    dispatcher: Arc<dyn ToolDispatcher>,
    cancellation: CancellationToken,
) -> Result<Router, OAuthProviderError> {
    let handler = handler::McpGatewayHandler::new(dispatcher)?;
    let host = provider
        .public_base_url()
        .host_str()
        .ok_or(OAuthProviderError::Configuration(
            "OAuth public base URL must include a host.",
        ))?
        .to_owned();
    let origin = provider.public_base_url().origin().ascii_serialization();
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_allowed_hosts([
            host,
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "::1".to_owned(),
        ])
        .with_allowed_origins([origin])
        .with_cancellation_token(cancellation);
    let service: StreamableHttpService<handler::McpGatewayHandler, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(handler.clone()), Arc::default(), config);
    let auth_provider = provider.clone();
    Ok(Router::new()
        .nest_service("/mcp", service)
        .layer(from_fn_with_state(auth_provider, auth::require_oauth)))
}
