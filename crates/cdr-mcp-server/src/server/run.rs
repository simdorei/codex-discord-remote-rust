use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::{GatewayConfig, GatewayConfigError, GatewayServerError, gateway_router};

#[derive(Debug, Error)]
pub enum GatewayRunError {
    #[error(transparent)]
    Config(#[from] GatewayConfigError),
    #[error(transparent)]
    Server(#[from] GatewayServerError),
    #[error("gateway listener failed: {0}")]
    Io(#[from] std::io::Error),
}

pub async fn run_from_env() -> Result<(), GatewayRunError> {
    run(GatewayConfig::from_env()?).await
}

pub async fn run(config: GatewayConfig) -> Result<(), GatewayRunError> {
    let address = config.bind_address();
    let cancellation = CancellationToken::new();
    let router = gateway_router(config, cancellation.clone())?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    eprintln!("rust_mcp_gateway_listening address={address}");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal(cancellation))
        .await?;
    Ok(())
}

async fn shutdown_signal(cancellation: CancellationToken) {
    #[cfg(unix)]
    {
        let terminate = async {
            let Ok(mut signal) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            else {
                std::future::pending::<()>().await;
                return;
            };
            signal.recv().await;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = terminate => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    cancellation.cancel();
}
