//! Historical discard is not admission of executable work.
use super::{MessageAdmissionError, MessageCandidate};
use cdr_store::StoreError;
use std::time::{SystemTime, UNIX_EPOCH};

/// Own only a deduplication marker, never an executable token or !new reservation.
/// The same unique `SQLite` row fences concurrent live admission.
pub(crate) fn discard_message_candidate_at(
    candidate: MessageCandidate,
    observed_at: SystemTime,
) -> Result<bool, MessageAdmissionError> {
    let parts = candidate.into_admission_parts();
    let now = observed_at
        .duration_since(UNIX_EPOCH)
        .map_err(StoreError::from)?
        .as_secs_f64();
    Ok(cdr_store::processed::claim(
        &parts.database,
        parts.persisted_id,
        now,
    )?)
}
