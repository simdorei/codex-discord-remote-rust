use thiserror::Error;

use super::{
    HISTORY_POLL_PAGE_LIMIT, HistoryClaimOutcome, HistoryClaimPurpose, HistoryPollCycleIo,
    HistoryPollItem, HistoryWatermark,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryGapCoverage {
    Reached,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryGapRecoveryOutcome {
    pub coverage: HistoryGapCoverage,
    pub fetched: usize,
    pub no_claim: usize,
    pub claim_attempted: usize,
    pub claim_won: usize,
    pub processed: usize,
}

#[derive(Debug, Error, PartialEq)]
pub enum HistoryGapRecoveryError<SourceError, AdaptationError, ClaimError, ProcessError> {
    #[error("history gap source failed: {0}")]
    Source(SourceError),
    #[error("history gap payload adaptation failed: {0}")]
    Adaptation(AdaptationError),
    #[error("history gap durable claim failed: {0}")]
    Claim(ClaimError),
    #[error("history gap item processing failed: {0}")]
    Process(ProcessError),
    #[error("Discord history batch has {actual} items; maximum is {HISTORY_POLL_PAGE_LIMIT}")]
    BatchTooLarge { actual: usize },
}

pub type HistoryGapRecoveryResult<Io> = Result<
    HistoryGapRecoveryOutcome,
    HistoryGapRecoveryError<
        <Io as HistoryPollCycleIo>::SourceError,
        <Io as HistoryPollCycleIo>::AdaptationError,
        <Io as HistoryPollCycleIo>::ClaimError,
        <Io as HistoryPollCycleIo>::ProcessError,
    >,
>;

/// Recovers the fixed newest page at or after an inclusive dropped-message floor.
///
/// `Incomplete` means the full ten-message page never reached the floor. Callers must retain the
/// sticky gap and expose degraded recovery instead of acknowledging it as complete.
pub async fn run_history_gap_recovery<Io>(
    channel_id: u64,
    floor: HistoryWatermark,
    io: &mut Io,
) -> HistoryGapRecoveryResult<Io>
where
    Io: HistoryPollCycleIo,
{
    let payloads = io
        .fetch(channel_id, HISTORY_POLL_PAGE_LIMIT)
        .await
        .map_err(HistoryGapRecoveryError::Source)?;
    if payloads.len() > HISTORY_POLL_PAGE_LIMIT {
        return Err(HistoryGapRecoveryError::BatchTooLarge {
            actual: payloads.len(),
        });
    }
    let fetched = payloads.len();
    let mut items = Vec::with_capacity(fetched);
    for payload in payloads {
        items.push(
            io.adapt(payload)
                .map_err(HistoryGapRecoveryError::Adaptation)?,
        );
    }
    let oldest = items.iter().filter_map(|item| item.watermark).min();
    let coverage = if fetched < HISTORY_POLL_PAGE_LIMIT || oldest.is_some_and(|item| item <= floor)
    {
        HistoryGapCoverage::Reached
    } else {
        HistoryGapCoverage::Incomplete
    };
    let mut outcome = HistoryGapRecoveryOutcome {
        coverage,
        fetched,
        no_claim: 0,
        claim_attempted: 0,
        claim_won: 0,
        processed: 0,
    };

    for item in items.into_iter().rev() {
        let Some(position) = item.watermark else {
            outcome.no_claim += 1;
            continue;
        };
        let HistoryPollItem::Candidate(item) = item.kind else {
            outcome.no_claim += 1;
            continue;
        };
        if position < floor {
            outcome.no_claim += 1;
            continue;
        }
        outcome.claim_attempted += 1;
        if let HistoryClaimOutcome::Won(admitted) = io
            .claim(item, HistoryClaimPurpose::Process)
            .map_err(HistoryGapRecoveryError::Claim)?
        {
            outcome.claim_won += 1;
            io.process(admitted)
                .await
                .map_err(HistoryGapRecoveryError::Process)?;
            outcome.processed += 1;
        }
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "gap_runner_tests.rs"]
mod tests;
