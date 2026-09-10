//! Process-local state used to decide which Discord history messages to deliver.

mod decision;
pub(crate) mod discord_adapter;
mod gap_runner;
mod runner;
mod state;
mod types;

pub use gap_runner::{
    HistoryGapCoverage, HistoryGapRecoveryError, HistoryGapRecoveryOutcome,
    HistoryGapRecoveryResult, run_history_gap_recovery,
};
pub use runner::{
    BoxHistoryPollFuture, HistoryClaimOutcome, HistoryPollCycleIo, HistoryPollCycleOutcome,
    HistoryPollCycleState, HistoryPollRunError, HistoryPollRunResult, run_begun_history_poll_cycle,
    run_history_poll_cycle,
};
pub use state::HistoryPollState;
pub use types::{
    HISTORY_POLL_PAGE_LIMIT, HistoryBatchItem, HistoryClaimPurpose, HistoryItemDecision,
    HistoryPollCommitError, HistoryPollCommitToken, HistoryPollCycleToken, HistoryPollItem,
    HistoryPollPhase, HistoryPollProposal, HistoryPollStateError, HistoryWatermark, PollStartedAt,
};
