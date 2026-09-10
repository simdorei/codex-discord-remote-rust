use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval};

use crate::action_executor::ActionExecutor;
use crate::queue_runner::TurnBackend;
use crate::restart_readiness::drain::{AdmissionGate, DrainGateError};

const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

pub async fn run_prompt_intake_recovery_worker<B: TurnBackend>(
    executor: Arc<ActionExecutor<B>>,
    shutdown: watch::Receiver<bool>,
) {
    run_prompt_intake_recovery_worker_with_admission(executor, AdmissionGate::new(), shutdown)
        .await;
}

pub async fn run_prompt_intake_recovery_worker_with_admission<B: TurnBackend>(
    executor: Arc<ActionExecutor<B>>,
    admission: AdmissionGate,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut retry = interval(RECOVERY_INTERVAL);
    retry.set_missed_tick_behavior(MissedTickBehavior::Delay);
    retry.tick().await;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = retry.tick() => {
                let _admission = match admission.try_enter() {
                    Ok(permit) => permit,
                    Err(DrainGateError::Sealed) => continue,
                    Err(error) => {
                        eprintln!("prompt_intake_recovery_admission_error: {error}");
                        continue;
                    }
                };
                if let Err(error) = executor.recover_prompt_intakes().await {
                    eprintln!("prompt_intake_recovery_error: {error}");
                }
            }
        }
    }
}
