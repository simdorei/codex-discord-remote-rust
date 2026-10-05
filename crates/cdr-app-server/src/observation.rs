//! Additive source-window API. Broadcast counts and payload equality are not sequence.
use crate::Notification;
pub const OBSERVATION_PAGE_SIZE: usize = 32;
pub const OBSERVATION_PAGE_BYTES: usize = 2 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct SourceObservation {
    pub sequence: u64,
    /// None retains the exact position of an oversized/not-copied payload.
    pub notification: Option<Notification>,
}
#[derive(Clone, Debug)]
pub struct ObservationWindow {
    pub owner_id: String,
    pub generation: u64,
    pub first_available: u64,
    pub source_upper: u64,
    pub upper: u64,
    pub scanned_through: u64,
    pub events: Vec<SourceObservation>,
}
