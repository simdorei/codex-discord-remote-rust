use std::path::Path;

use cdr_app_server::AppServerError;
use cdr_store::Result as StoreResult;
use cdr_store::claims::{release_busy_choice_claim, release_component_claim};

use crate::action_executor::ActionError;
use crate::queue_runner::QueueRunnerError;

use super::BusyComponentError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimFailureDisposition {
    Release,
    RetainIndeterminate,
}

#[must_use]
pub const fn app_server_claim_failure(error: &AppServerError) -> ClaimFailureDisposition {
    if matches!(
        error,
        AppServerError::Spawn { .. }
            | AppServerError::Remote { .. }
            | AppServerError::MissingPipe { .. }
            | AppServerError::GenerationMismatch { .. }
            | AppServerError::GenerationQuarantined { .. }
            | AppServerError::StaleServerRequest { .. }
    ) {
        ClaimFailureDisposition::Release
    } else {
        ClaimFailureDisposition::RetainIndeterminate
    }
}

#[must_use]
pub fn action_claim_failure(error: &ActionError) -> ClaimFailureDisposition {
    match error {
        ActionError::AppServer(error) => app_server_claim_failure(error),
        ActionError::Queue(QueueRunnerError::Backend(error)) => {
            if error.ambiguous {
                ClaimFailureDisposition::RetainIndeterminate
            } else {
                ClaimFailureDisposition::Release
            }
        }
        ActionError::Queue(
            QueueRunnerError::IntegerRange
            | QueueRunnerError::SystemTime(_)
            | QueueRunnerError::LockPoisoned,
        )
        | ActionError::NoTarget
        | ActionError::IntegerRange => ClaimFailureDisposition::Release,
        _ => ClaimFailureDisposition::RetainIndeterminate,
    }
}

#[must_use]
pub fn busy_action_claim_failure(error: &BusyComponentError) -> ClaimFailureDisposition {
    match error {
        BusyComponentError::Action(error) => action_claim_failure(error),
        BusyComponentError::AppServer(error) => app_server_claim_failure(error),
        BusyComponentError::NoActiveTurn
        | BusyComponentError::ControlNotDispatched(_)
        | BusyComponentError::NoTarget
        | BusyComponentError::SteerNotAllowed => ClaimFailureDisposition::Release,
        _ => ClaimFailureDisposition::RetainIndeterminate,
    }
}

pub fn retain_or_release_component_claim(
    database: &Path,
    claim_id: &str,
    error: &AppServerError,
) -> StoreResult<ClaimFailureDisposition> {
    let disposition = app_server_claim_failure(error);
    if disposition == ClaimFailureDisposition::Release {
        let _ = release_component_claim(database, claim_id)?;
    }
    Ok(disposition)
}

pub fn retain_or_release_busy_claim(
    database: &Path,
    choice_id: &str,
    error: &BusyComponentError,
) -> StoreResult<ClaimFailureDisposition> {
    let disposition = busy_action_claim_failure(error);
    if disposition == ClaimFailureDisposition::Release {
        let _ = release_busy_choice_claim(database, choice_id)?;
    }
    Ok(disposition)
}
