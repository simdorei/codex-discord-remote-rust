use std::path::PathBuf;
use std::sync::Arc;

use cdr_app_server::ResidentAppServer;
use tokio::time::{Duration, MissedTickBehavior, interval};

use super::DiscordRuntimeError;
use crate::restart_readiness::drain::DrainFenceKey;
use crate::restart_readiness::drain_controller::RuntimeDrainController;
use crate::restart_readiness::live_drain::LiveDrainServer;

#[path = "gateway_drain.rs"]
mod gateway_drain;
use gateway_drain::drain_until_quiescent;

#[derive(Debug)]
pub(super) enum GatewayLoopOutcome {
    Shutdown,
    Drained(DrainFenceKey),
}

pub(super) struct GatewayLoopContext<S = ResidentAppServer> {
    pub operation_root: PathBuf,
    pub drain: Arc<RuntimeDrainController>,
    pub server: Arc<S>,
    pub mirror_db: PathBuf,
}

pub(super) async fn run_gateway_loop<S: LiveDrainServer>(
    context: GatewayLoopContext<S>,
) -> Result<GatewayLoopOutcome, DiscordRuntimeError> {
    let mut operation_poll = interval(Duration::from_millis(250));
    operation_poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut reported_prepare_failure = None;
    loop {
        tokio::select! {
            _ = operation_poll.tick() => {
                if crate::operation_marker::stop_requested(&context.operation_root) {
                    eprintln!("rust_runtime_graceful_shutdown_requested");
                    return Ok(GatewayLoopOutcome::Shutdown);
                }
                match context.drain.claim_prepare() {
                    Ok(Some(key)) => {
                        eprintln!("rust_runtime_restart_drain_sealed nonce={}", key.nonce());
                        return Ok(drain_until_quiescent(&context, key).await);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let quarantine = context.drain.quarantine_untrusted_prepare();
                        let failure = quarantine.map_or_else(
                            |quarantine_error| {
                                format!(
                                    "claim={error}; quarantine={quarantine_error}; runtime_remains_sealed=true"
                                )
                            },
                            |()| format!("claim={error}; runtime_remains_sealed=true"),
                        );
                        if reported_prepare_failure.as_ref() != Some(&failure) {
                            eprintln!("restart_drain_prepare_claim_failed: {failure}");
                            reported_prepare_failure = Some(failure);
                        }
                    }
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(DiscordRuntimeError::CtrlC)?;
                return Ok(GatewayLoopOutcome::Shutdown);
            }
        }
    }
}
