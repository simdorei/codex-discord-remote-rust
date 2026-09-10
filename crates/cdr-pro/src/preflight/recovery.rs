use crate::diagnostics::{DiagnosticCode, ProError, Result};
use std::fmt::Write as _;

use super::RuntimeStatus;

pub fn recover_stale<C, R>(mut check: C, mut refresh: R) -> Result<RuntimeStatus>
where
    C: FnMut() -> Result<RuntimeStatus>,
    R: FnMut() -> std::result::Result<bool, String>,
{
    let error = match check() {
        Ok(status) => return Ok(status),
        Err(error) => error,
    };
    let is_stale = error
        .diagnostic()
        .is_some_and(|diagnostic| diagnostic.code == DiagnosticCode::ResidentStale);
    if !is_stale {
        return Err(error);
    }
    match refresh() {
        Ok(true) => check(),
        Ok(false) => Err(error),
        Err(refresh_error) => {
            let ProError::Preflight(mut diagnostic) = error;
            let _ = write!(
                diagnostic.internal_detail,
                "; automatic resident refresh failed error={refresh_error}"
            );
            Err(ProError::Preflight(diagnostic))
        }
    }
}
