//! Fail-closed live drain state machine entered after an exact prepare claim.

use std::path::Path;

use tokio::time::{Duration, Instant, sleep};

use super::{GatewayLoopContext, GatewayLoopOutcome};
use crate::restart_readiness::drain::{AdmissionGate, DrainFenceKey, DrainGateError};
use crate::restart_readiness::live_drain::{
    LiveDrainError, LiveDrainServer, LiveDrainState, check_runtime_quiescence,
};

const ADMISSION_DRAIN_TIMEOUT: Duration = Duration::from_mins(15);
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DRAIN_ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(5);

pub(super) async fn drain_until_quiescent<S: LiveDrainServer>(
    context: &GatewayLoopContext<S>,
    key: DrainFenceKey,
) -> GatewayLoopOutcome {
    let gate = context.drain.admission_gate();
    drain_until_quiescent_inner(
        &context.operation_root,
        &gate,
        &context.mirror_db,
        context.server.as_ref(),
        key,
        ADMISSION_DRAIN_TIMEOUT,
    )
    .await
}

async fn drain_until_quiescent_inner(
    operation_root: &Path,
    gate: &AdmissionGate,
    mirror_db: &Path,
    server: &impl LiveDrainServer,
    key: DrainFenceKey,
    timeout: Duration,
) -> GatewayLoopOutcome {
    let mut last_reason = "resident app-server has not been inspected".to_owned();
    loop {
        match wait_for_admission(operation_root, gate, &key, timeout).await {
            AdmissionWait::Drained => {}
            AdmissionWait::Shutdown => return GatewayLoopOutcome::Shutdown,
            AdmissionWait::TimedOut => {
                eprintln!("restart_drain_still_waiting phase=admission reason={last_reason}");
                continue;
            }
            AdmissionWait::GateError(error) => {
                eprintln!("restart_drain_admission_check_failed: {error}");
                if pause_or_shutdown(operation_root, DRAIN_ERROR_RETRY_INTERVAL).await {
                    return GatewayLoopOutcome::Shutdown;
                }
                continue;
            }
        }
        let live_state = match inspect_or_shutdown(operation_root, mirror_db, server).await {
            LiveInspection::Shutdown => return GatewayLoopOutcome::Shutdown,
            LiveInspection::Finished(Ok(state)) => state,
            LiveInspection::Finished(Err(error)) => {
                eprintln!("restart_drain_live_check_failed: {error}");
                if pause_or_shutdown(operation_root, DRAIN_ERROR_RETRY_INTERVAL).await {
                    return GatewayLoopOutcome::Shutdown;
                }
                continue;
            }
        };
        match live_state {
            LiveDrainState::Blocked { reason } => last_reason = reason,
            LiveDrainState::Ready => {
                if let Err(error) = gate.close_controls(&key) {
                    eprintln!("restart_drain_close_controls_failed: {error}");
                    if pause_or_shutdown(operation_root, DRAIN_ERROR_RETRY_INTERVAL).await {
                        return GatewayLoopOutcome::Shutdown;
                    }
                    continue;
                }
                match wait_for_admission(operation_root, gate, &key, timeout).await {
                    AdmissionWait::Shutdown => return GatewayLoopOutcome::Shutdown,
                    AdmissionWait::TimedOut => {
                        reopen_controls(gate, &key, "final admission timeout");
                        continue;
                    }
                    AdmissionWait::GateError(error) => {
                        eprintln!("restart_drain_final_admission_check_failed: {error}");
                        reopen_controls(gate, &key, "final admission check failure");
                        if pause_or_shutdown(operation_root, DRAIN_ERROR_RETRY_INTERVAL).await {
                            return GatewayLoopOutcome::Shutdown;
                        }
                        continue;
                    }
                    AdmissionWait::Drained => {}
                }
                match inspect_or_shutdown(operation_root, mirror_db, server).await {
                    LiveInspection::Shutdown => return GatewayLoopOutcome::Shutdown,
                    LiveInspection::Finished(Ok(LiveDrainState::Ready)) => {
                        return GatewayLoopOutcome::Drained(key);
                    }
                    LiveInspection::Finished(Ok(LiveDrainState::Blocked { reason })) => {
                        last_reason = reason;
                        reopen_controls(gate, &key, &last_reason);
                    }
                    LiveInspection::Finished(Err(error)) => {
                        eprintln!("restart_drain_final_live_check_failed: {error}");
                        reopen_controls(gate, &key, "final live check failure");
                        if pause_or_shutdown(operation_root, DRAIN_ERROR_RETRY_INTERVAL).await {
                            return GatewayLoopOutcome::Shutdown;
                        }
                        continue;
                    }
                }
            }
        }
        if pause_or_shutdown(operation_root, DRAIN_POLL_INTERVAL).await {
            return GatewayLoopOutcome::Shutdown;
        }
    }
}

enum AdmissionWait {
    Drained,
    TimedOut,
    GateError(DrainGateError),
    Shutdown,
}

async fn wait_for_admission(
    operation_root: &Path,
    gate: &AdmissionGate,
    key: &DrainFenceKey,
    timeout: Duration,
) -> AdmissionWait {
    let deadline = Instant::now() + timeout;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut ctrl_c_enabled = true;
    loop {
        if crate::operation_marker::stop_requested(operation_root) {
            return AdmissionWait::Shutdown;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return AdmissionWait::TimedOut;
        }
        let slice = remaining.min(DRAIN_POLL_INTERVAL);
        tokio::select! {
            result = gate.wait_drained(key, slice) => match result {
                Ok(()) => return AdmissionWait::Drained,
                Err(DrainGateError::Timeout { .. }) => {}
                Err(error) => return AdmissionWait::GateError(error),
            },
            signal = &mut ctrl_c, if ctrl_c_enabled => match signal {
                Ok(()) => return AdmissionWait::Shutdown,
                Err(error) => {
                    ctrl_c_enabled = false;
                    eprintln!("restart_drain_ctrl_c_listener_failed: {error}");
                }
            },
        }
    }
}

enum LiveInspection {
    Finished(Result<LiveDrainState, LiveDrainError>),
    Shutdown,
}

async fn inspect_or_shutdown(
    operation_root: &Path,
    mirror_db: &Path,
    server: &impl LiveDrainServer,
) -> LiveInspection {
    let inspection = check_runtime_quiescence(mirror_db, server);
    tokio::pin!(inspection);
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut ctrl_c_enabled = true;
    loop {
        tokio::select! {
            result = &mut inspection => return LiveInspection::Finished(result),
            () = sleep(DRAIN_POLL_INTERVAL) => {
                if crate::operation_marker::stop_requested(operation_root) {
                    return LiveInspection::Shutdown;
                }
            }
            signal = &mut ctrl_c, if ctrl_c_enabled => match signal {
                Ok(()) => return LiveInspection::Shutdown,
                Err(error) => {
                    ctrl_c_enabled = false;
                    eprintln!("restart_drain_ctrl_c_listener_failed: {error}");
                }
            },
        }
    }
}

async fn pause_or_shutdown(operation_root: &Path, delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut ctrl_c_enabled = true;
    loop {
        if crate::operation_marker::stop_requested(operation_root) {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        tokio::select! {
            () = sleep(remaining.min(DRAIN_POLL_INTERVAL)) => {}
            signal = &mut ctrl_c, if ctrl_c_enabled => match signal {
                Ok(()) => return true,
                Err(error) => {
                    ctrl_c_enabled = false;
                    eprintln!("restart_drain_ctrl_c_listener_failed: {error}");
                }
            }
        }
    }
}

fn reopen_controls(gate: &AdmissionGate, key: &DrainFenceKey, reason: &str) {
    match gate.open_controls(key) {
        Ok(()) => eprintln!("restart_drain_controls_reopened reason={reason}"),
        Err(error) => eprintln!("restart_drain_reopen_controls_failed: {error}"),
    }
}

#[cfg(test)]
#[path = "gateway_loop_tests.rs"]
mod tests;
