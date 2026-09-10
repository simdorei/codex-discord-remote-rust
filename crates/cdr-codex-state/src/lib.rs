mod context_batch;
mod context_read;
mod context_text;
mod context_usage;
mod error;
mod paths;
mod rollout;
mod session_index;
mod store;
mod tail;
mod thread;

pub use context_batch::{
    ContextBatch, ContextEntry, ContextReadTarget, read_context_batch,
    read_context_batch_with_mode, read_context_batch_with_recent,
};
pub use context_read::{
    ContextReadBudget, ContextReadError, ContextSnapshot, read_context_snapshot,
    read_context_snapshot_with_mode, read_context_usage,
};
pub use context_text::{ContextTextItem, RecentTextMode};
pub use context_usage::{ContextUsage, ContextUsageError, context_usage_from_events};
pub use error::CodexStateError;
pub use paths::resolve_state_db_path;
pub use rollout::{load_missing_vscode_rollout_threads, parse_rollout_thread};
pub use session_index::{
    build_ui_name_prefixes, load_active_workspace_roots, load_session_thread_names,
    normalize_ui_match_text, normalize_workspace_path, strip_windows_extended_prefix,
};
pub use store::CodexThreadStore;
pub use tail::{SessionTail, iter_session_events, read_new_session_events};
pub use thread::ThreadInfo;
pub use thread_ref::{ThreadResolveError, resolve_thread_ref, workspace_ref_map};

mod thread_ref;
