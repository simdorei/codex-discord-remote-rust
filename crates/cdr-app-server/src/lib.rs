mod approval_replies;
mod client;
mod contract;
mod control;
mod dead_generation;
mod diagnostics;
mod error;
pub mod goal;
mod input_replies;
mod input_validation;
mod manager;
pub mod outcomes;
mod process;
pub mod requests;
mod rpc;
mod startup_budget;
mod state;
mod transport;

pub use approval_replies::{ApprovalAnswer, build_approval_response, parse_approval_answer};
pub use client::{AppServerClient, AppServerConfig, DEFAULT_CLIENT_NAME, DEFAULT_CLIENT_TITLE};
pub use contract::{USED_CLIENT_REQUESTS, USED_NOTIFICATIONS, USED_SERVER_REQUESTS};
pub use dead_generation::{
    DeadActiveTurn, DeadGenerationFence, DeadGenerationSettleResult, DeadGenerationWork,
    DeadServerRequest,
};
pub use diagnostics::DiagnosticSnapshot;
pub use error::{AppServerError, RpcErrorPayload};
pub use input_replies::{
    InputResponse, build_input_response, resolve_input_answers, split_input_values,
};
pub use input_validation::{input_option_labels, validate_input_questions};
pub use manager::{
    ResidentAppServer, ResidentLifecycleSnapshot, ResidentNotificationEvent,
    ResidentServerRequestEvent,
};
pub use rpc::{Notification, RequestId, ServerRequest, ServerRequestOccurrence};
pub use startup_budget::{APP_SERVER_INITIALIZE_TIMEOUT, APP_SERVER_STARTUP_TIMEOUT};
pub use state::{LifecycleSnapshot, extract_thread_id, extract_turn_id};
