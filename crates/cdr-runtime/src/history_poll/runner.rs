use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use super::{
    HISTORY_POLL_PAGE_LIMIT, HistoryBatchItem, HistoryClaimPurpose, HistoryItemDecision,
    HistoryPollCommitError, HistoryPollCommitToken, HistoryPollCycleToken, HistoryPollPhase,
    HistoryPollProposal, HistoryPollState, HistoryPollStateError, PollStartedAt,
};

pub type BoxHistoryPollFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// External work needed by one history-poll channel cycle.
pub trait HistoryPollCycleIo {
    type Payload;
    type Item;
    /// Opaque proof that the durable claim was won, consumed by `process`.
    type Admitted;
    type SourceError;
    type AdaptationError;
    type ClaimError;
    type ProcessError;

    fn fetch(
        &mut self,
        channel_id: u64,
        limit: usize,
    ) -> BoxHistoryPollFuture<'_, Vec<Self::Payload>, Self::SourceError>;

    /// Produces an item that owns everything needed by later claim and process steps.
    ///
    /// The payload is consumed, so an adapter may retain the complete payload inside `Item`, but
    /// it cannot return an item borrowing from that payload.
    fn adapt(
        &mut self,
        payload: Self::Payload,
    ) -> Result<HistoryBatchItem<Self::Item>, Self::AdaptationError>;

    fn claim(
        &mut self,
        item: Self::Item,
        purpose: HistoryClaimPurpose,
    ) -> Result<HistoryClaimOutcome<Self::Admitted>, Self::ClaimError>;

    fn process(
        &mut self,
        admitted: Self::Admitted,
    ) -> BoxHistoryPollFuture<'_, (), Self::ProcessError>;
}

/// Result of consuming an unclaimed item at the durable admission boundary.
#[derive(Debug, Eq, PartialEq)]
pub enum HistoryClaimOutcome<Admitted> {
    /// Another ingress path already owns the item; suppress it successfully.
    Lost,
    /// This cycle owns the claim and receives the only value accepted by `process`.
    Won(Admitted),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryPollCycleOutcome {
    pub phase: HistoryPollPhase,
    pub fetched: usize,
    pub no_claim: usize,
    pub claim_attempted: usize,
    pub claim_won: usize,
    /// Items deliberately terminal as `ClaimAndDiscard`, whether or not their claim was won.
    pub discarded: usize,
    pub processed: usize,
}

/// State boundary used by the runner; `HistoryPollState` is the production implementation.
pub trait HistoryPollCycleState {
    fn begin(
        &mut self,
        channel_id: u64,
        poll_started_at: PollStartedAt,
    ) -> Result<HistoryPollCycleToken, HistoryPollStateError>;

    fn propose<T>(
        &self,
        cycle: &HistoryPollCycleToken,
        newest_first: Vec<HistoryBatchItem<T>>,
    ) -> Result<HistoryPollProposal<T>, HistoryPollStateError>;

    fn commit(
        &mut self,
        channel_id: u64,
        token: &HistoryPollCommitToken,
    ) -> Result<(), HistoryPollCommitError>;
}

impl HistoryPollCycleState for HistoryPollState {
    fn begin(
        &mut self,
        channel_id: u64,
        poll_started_at: PollStartedAt,
    ) -> Result<HistoryPollCycleToken, HistoryPollStateError> {
        Self::begin(self, channel_id, poll_started_at)
    }

    fn propose<T>(
        &self,
        cycle: &HistoryPollCycleToken,
        newest_first: Vec<HistoryBatchItem<T>>,
    ) -> Result<HistoryPollProposal<T>, HistoryPollStateError> {
        Self::propose(self, cycle, newest_first)
    }

    fn commit(
        &mut self,
        channel_id: u64,
        token: &HistoryPollCommitToken,
    ) -> Result<(), HistoryPollCommitError> {
        Self::commit(self, channel_id, token)
    }
}

#[derive(Debug, Eq, Error, PartialEq)]
pub enum HistoryPollRunError<SourceError, AdaptationError, ClaimError, ProcessError> {
    #[error("history source failed: {0}")]
    Source(SourceError),
    #[error("history payload adaptation failed: {0}")]
    Adaptation(AdaptationError),
    #[error("history durable claim failed: {0}")]
    Claim(ClaimError),
    #[error("history item processing failed: {0}")]
    Process(ProcessError),
    #[error("history poll state failed: {0}")]
    State(HistoryPollStateError),
    #[error("history poll commit failed: {0}")]
    Commit(HistoryPollCommitError),
}

pub type HistoryPollRunResult<Io> = Result<
    HistoryPollCycleOutcome,
    HistoryPollRunError<
        <Io as HistoryPollCycleIo>::SourceError,
        <Io as HistoryPollCycleIo>::AdaptationError,
        <Io as HistoryPollCycleIo>::ClaimError,
        <Io as HistoryPollCycleIo>::ProcessError,
    >,
>;

pub async fn run_history_poll_cycle<State, Io>(
    state: &mut State,
    channel_id: u64,
    poll_started_at: PollStartedAt,
    io: &mut Io,
) -> HistoryPollRunResult<Io>
where
    State: HistoryPollCycleState,
    Io: HistoryPollCycleIo,
{
    let cycle = state
        .begin(channel_id, poll_started_at)
        .map_err(HistoryPollRunError::State)?;
    run_begun_history_poll_cycle(state, channel_id, cycle, io).await
}

/// Runs a cycle whose fixed boundary has already been installed in `state`.
///
/// This split lets callers persist the first-poll boundary before fallible policy loading.
pub async fn run_begun_history_poll_cycle<State, Io>(
    state: &mut State,
    channel_id: u64,
    cycle: HistoryPollCycleToken,
    io: &mut Io,
) -> HistoryPollRunResult<Io>
where
    State: HistoryPollCycleState,
    Io: HistoryPollCycleIo,
{
    let payloads = io
        .fetch(channel_id, HISTORY_POLL_PAGE_LIMIT)
        .await
        .map_err(HistoryPollRunError::Source)?;
    if payloads.len() > HISTORY_POLL_PAGE_LIMIT {
        return Err(HistoryPollRunError::State(
            HistoryPollStateError::BatchTooLarge {
                actual: payloads.len(),
            },
        ));
    }

    let fetched = payloads.len();
    let mut items = Vec::with_capacity(fetched);
    for payload in payloads {
        items.push(io.adapt(payload).map_err(HistoryPollRunError::Adaptation)?);
    }
    let proposal = state
        .propose(&cycle, items)
        .map_err(HistoryPollRunError::State)?;
    let (phase, _, decisions, commit_token) = proposal.into_parts();
    let mut outcome = HistoryPollCycleOutcome {
        phase,
        fetched,
        no_claim: 0,
        claim_attempted: 0,
        claim_won: 0,
        discarded: 0,
        processed: 0,
    };

    for decision in decisions {
        match decision {
            HistoryItemDecision::NoClaim => outcome.no_claim += 1,
            HistoryItemDecision::ClaimAndDiscard(item) => {
                outcome.claim_attempted += 1;
                if let HistoryClaimOutcome::Won(_admitted) = io
                    .claim(item, HistoryClaimPurpose::Discard)
                    .map_err(HistoryPollRunError::Claim)?
                {
                    outcome.claim_won += 1;
                }
                outcome.discarded += 1;
            }
            HistoryItemDecision::ClaimAndProcess(item) => {
                outcome.claim_attempted += 1;
                if let HistoryClaimOutcome::Won(admitted) = io
                    .claim(item, HistoryClaimPurpose::Process)
                    .map_err(HistoryPollRunError::Claim)?
                {
                    outcome.claim_won += 1;
                    io.process(admitted)
                        .await
                        .map_err(HistoryPollRunError::Process)?;
                    outcome.processed += 1;
                }
            }
        }
    }

    state
        .commit(channel_id, &commit_token)
        .map_err(HistoryPollRunError::Commit)?;
    Ok(outcome)
}

#[cfg(test)]
#[path = "runner_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
