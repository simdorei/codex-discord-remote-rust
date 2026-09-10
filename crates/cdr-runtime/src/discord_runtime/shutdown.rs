use cdr_app_server::AppServerError;
use cdr_discord::gateway::{GatewayShutdownError, GatewayShutdownReport};

use super::DiscordRuntimeError;
use super::gateway_loop::GatewayLoopOutcome;
use super::worker_supervision::WorkerShutdownReport;

pub(super) enum ShutdownCause {
    Control(Result<GatewayLoopOutcome, DiscordRuntimeError>),
    Worker(&'static str),
    WorkerMonitorClosed,
    GatewayShard(u32),
    GatewayMonitorClosed,
}

impl ShutdownCause {
    pub(super) fn record_before_cleanup(&self) {
        match self {
            Self::Control(Err(error)) => {
                eprintln!("primary_runtime_shutdown_error source=control error={error}");
            }
            Self::Worker(worker) => {
                eprintln!("primary_runtime_shutdown_cause worker={worker}");
            }
            Self::WorkerMonitorClosed => {
                eprintln!("primary_runtime_shutdown_cause worker-monitor-closed");
            }
            Self::GatewayShard(shard) => {
                eprintln!("primary_runtime_shutdown_cause gateway-shard={shard}");
            }
            Self::GatewayMonitorClosed => {
                eprintln!("primary_runtime_shutdown_cause gateway-monitor-closed");
            }
            Self::Control(Ok(_)) => {}
        }
    }

    pub(super) fn non_heartbeat_worker(&self) -> Option<&'static str> {
        match self {
            Self::Worker(worker) if *worker != "heartbeat" => Some(*worker),
            _ => None,
        }
    }

    pub(super) fn gateway_shard(&self) -> Option<u32> {
        match self {
            Self::GatewayShard(shard) => Some(*shard),
            _ => None,
        }
    }

    pub(super) fn propagate(
        self,
        gateway: GatewayShutdownReport,
        workers: WorkerShutdownReport,
        heartbeat: Result<(), DiscordRuntimeError>,
        server_close: Result<(), AppServerError>,
    ) -> Result<(), DiscordRuntimeError> {
        let (worker_trigger, worker_cleanup) = workers.into_parts();
        let (gateway_trigger, gateway_cleanup) = gateway.into_parts();
        match self {
            Self::Control(Ok(_)) => {
                worker_cleanup?;
                gateway_cleanup?;
                heartbeat?;
                server_close?;
                Ok(())
            }
            Self::Control(Err(error)) => finish_primary(
                error,
                worker_cleanup,
                gateway_cleanup,
                heartbeat,
                server_close,
            ),
            Self::Worker("heartbeat") => {
                let primary = heartbeat
                    .err()
                    .unwrap_or(DiscordRuntimeError::WorkerExited {
                        worker: "heartbeat",
                    });
                finish_primary(
                    primary,
                    worker_cleanup,
                    gateway_cleanup,
                    Ok(()),
                    server_close,
                )
            }
            Self::Worker(worker) => {
                let primary = worker_trigger
                    .and_then(Result::err)
                    .unwrap_or(DiscordRuntimeError::WorkerExited { worker });
                finish_primary(
                    primary,
                    worker_cleanup,
                    gateway_cleanup,
                    heartbeat,
                    server_close,
                )
            }
            Self::WorkerMonitorClosed => finish_primary(
                DiscordRuntimeError::WorkerMonitorClosed,
                worker_cleanup,
                gateway_cleanup,
                heartbeat,
                server_close,
            ),
            Self::GatewayShard(shard) => {
                let primary = gateway_trigger.and_then(Result::err).map_or(
                    DiscordRuntimeError::GatewayShardExited { shard },
                    Into::into,
                );
                finish_primary(
                    primary,
                    worker_cleanup,
                    gateway_cleanup,
                    heartbeat,
                    server_close,
                )
            }
            Self::GatewayMonitorClosed => finish_primary(
                DiscordRuntimeError::GatewayShardMonitorClosed,
                worker_cleanup,
                gateway_cleanup,
                heartbeat,
                server_close,
            ),
        }
    }
}

pub(super) fn validate_drain_cleanup(
    gateway: GatewayShutdownReport,
    workers: WorkerShutdownReport,
    server_close: Result<(), AppServerError>,
) -> Result<(), DiscordRuntimeError> {
    let (_, worker_cleanup) = workers.into_parts();
    let (_, gateway_cleanup) = gateway.into_parts();
    worker_cleanup?;
    gateway_cleanup?;
    server_close?;
    Ok(())
}

fn finish_primary(
    primary: DiscordRuntimeError,
    workers: Result<(), DiscordRuntimeError>,
    gateway: Result<(), GatewayShutdownError>,
    heartbeat: Result<(), DiscordRuntimeError>,
    server_close: Result<(), AppServerError>,
) -> Result<(), DiscordRuntimeError> {
    log_secondary("workers", workers.err());
    log_secondary("gateway", gateway.err());
    log_secondary("heartbeat", heartbeat.err());
    log_secondary("app-server", server_close.err());
    Err(primary)
}

fn log_secondary(label: &str, error: Option<impl std::fmt::Display>) {
    if let Some(error) = error {
        eprintln!("secondary_runtime_shutdown_error component={label} error={error}");
    }
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;
