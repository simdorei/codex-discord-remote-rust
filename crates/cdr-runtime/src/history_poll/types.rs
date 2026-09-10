use thiserror::Error;

pub const HISTORY_POLL_PAGE_LIMIT: usize = 10;

/// A UTC instant captured immediately before channel lookup and fetching.
///
/// The adapter must normalize aware timestamps to UTC and treat naive timestamps as UTC before
/// converting them to microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PollStartedAt {
    normalized_utc_micros: i64,
}

impl PollStartedAt {
    #[must_use]
    pub const fn from_normalized_utc_micros(normalized_utc_micros: i64) -> Self {
        Self {
            normalized_utc_micros,
        }
    }

    pub(super) const fn watermark(self) -> HistoryWatermark {
        HistoryWatermark {
            normalized_utc_micros: self.normalized_utc_micros,
            message_id: 0,
        }
    }
}

/// A valid Discord message position, ordered by UTC creation time and then message ID.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct HistoryWatermark {
    normalized_utc_micros: i64,
    message_id: u64,
}

impl HistoryWatermark {
    /// Builds a key after the adapter has parsed and normalized the Discord timestamp.
    #[must_use]
    pub const fn from_message(normalized_utc_micros: i64, message_id: u64) -> Option<Self> {
        if message_id == 0 {
            return None;
        }
        Some(Self {
            normalized_utc_micros,
            message_id,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryBatchItem<T> {
    pub watermark: Option<HistoryWatermark>,
    /// The adapter's canonical message-plan result, with work carried only by candidates.
    pub kind: HistoryPollItem<T>,
}

/// An adapted history item before its watermark is compared with the cycle boundary.
///
/// `Ignore` deliberately has no payload, so ignored work cannot be constructed and later handed
/// to the durable claim boundary.
///
/// ```compile_fail
/// use cdr_runtime::history_poll::HistoryPollItem;
///
/// let _: HistoryPollItem<&str> = HistoryPollItem::Ignore("must not reach claim");
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoryPollItem<T> {
    /// Never claim this item. Its valid watermark still advances the channel cursor.
    Ignore,
    /// A non-ignored plan or plan error that may need a durable claim before any side effect.
    Candidate(T),
}

/// The only values the runner may execute after a side-effect-free proposal.
///
/// `NoClaim` deliberately has no payload. Only the two claim-bearing variants can supply `T` to
/// `HistoryPollCycleIo::claim`.
///
/// ```compile_fail
/// use cdr_runtime::history_poll::HistoryItemDecision;
///
/// let _: HistoryItemDecision<&str> = HistoryItemDecision::NoClaim("must not reach claim");
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoryItemDecision<T> {
    /// Do not attempt a durable claim; no claimable value exists in this state.
    NoClaim,
    /// Win the durable claim, if possible, solely to suppress a late gateway copy.
    ClaimAndDiscard(T),
    /// Only process the item if the subsequent durable claim succeeds.
    ClaimAndProcess(T),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryClaimPurpose {
    Discard,
    Process,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryPollPhase {
    Prime,
    Reprime,
    Incremental,
}

#[derive(Debug, Eq, PartialEq)]
pub struct HistoryPollProposal<T> {
    pub phase: HistoryPollPhase,
    pub next_watermark: HistoryWatermark,
    pub items_oldest_first: Vec<HistoryItemDecision<T>>,
    commit_token: HistoryPollCommitToken,
}

impl<T> HistoryPollProposal<T> {
    #[must_use]
    pub const fn commit_token(&self) -> &HistoryPollCommitToken {
        &self.commit_token
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        HistoryPollPhase,
        HistoryWatermark,
        Vec<HistoryItemDecision<T>>,
        HistoryPollCommitToken,
    ) {
        (
            self.phase,
            self.next_watermark,
            self.items_oldest_first,
            self.commit_token,
        )
    }

    pub(super) const fn new(
        phase: HistoryPollPhase,
        next_watermark: HistoryWatermark,
        items_oldest_first: Vec<HistoryItemDecision<T>>,
        commit_token: HistoryPollCommitToken,
    ) -> Self {
        Self {
            phase,
            next_watermark,
            items_oldest_first,
            commit_token,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct HistoryPollCommitToken {
    pub(super) channel_id: u64,
    pub(super) expected: ExpectedChannelState,
    pub(super) next_watermark: HistoryWatermark,
}

#[derive(Debug, Eq, PartialEq)]
pub struct HistoryPollCycleToken {
    pub(super) channel_id: u64,
    pub(super) expected: ExpectedChannelState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExpectedChannelState {
    Priming {
        phase: HistoryPollPhase,
        boundary: HistoryWatermark,
        revision: u64,
    },
    Active {
        watermark: Option<HistoryWatermark>,
        revision: u64,
    },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum HistoryPollStateError {
    #[error("Discord history batch has {actual} items; maximum is {HISTORY_POLL_PAGE_LIMIT}")]
    BatchTooLarge { actual: usize },
    #[error("history poll cycle for channel {channel_id} is stale")]
    StaleCycle { channel_id: u64 },
    #[error("history state revision exhausted for channel {channel_id}")]
    RevisionExhausted { channel_id: u64 },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum HistoryPollCommitError {
    #[error("history proposal belongs to channel {token_channel_id}, not {requested_channel_id}")]
    ChannelMismatch {
        token_channel_id: u64,
        requested_channel_id: u64,
    },
    #[error("history proposal for channel {channel_id} is stale or already committed")]
    Stale { channel_id: u64 },
    #[error("history state revision exhausted for channel {channel_id}")]
    RevisionExhausted { channel_id: u64 },
}
