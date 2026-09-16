//! Durable `SQLite` state shared by the Rust Discord runtime.

pub mod archive_fence;
pub mod backup;
pub mod claims;
pub mod commentary_outbox;
pub mod control_binding;
pub mod dead_generation;
pub mod delivery;
pub mod delivery_receipt;
mod error;
pub mod first_reply;
pub mod goal_progress;
pub mod history;
pub mod ingress;
pub mod mapping;
pub mod mirror;
pub mod new_reply;
pub mod observed_completion;
pub mod observed_final_answer;
pub mod processed;
pub mod prompt_intake;
pub mod queue;
pub mod restart_readiness;
pub mod room_cleanup;
pub mod reserve_policy;
pub mod schema;

pub use error::{Result, StoreError};
