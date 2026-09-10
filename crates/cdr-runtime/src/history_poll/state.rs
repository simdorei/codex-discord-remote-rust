use std::collections::{HashMap, HashSet};

use super::decision::{decide_items, proposal_parameters};
use super::types::{
    ExpectedChannelState, HISTORY_POLL_PAGE_LIMIT, HistoryBatchItem, HistoryPollCommitError,
    HistoryPollCommitToken, HistoryPollCycleToken, HistoryPollPhase, HistoryPollProposal,
    HistoryPollStateError, HistoryWatermark, PollStartedAt,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChannelState {
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

/// Non-durable history cursor state. A new instance deliberately re-primes all channels.
#[derive(Debug, Default)]
pub struct HistoryPollState {
    channels: HashMap<u64, ChannelState>,
    last_revision: u64,
}

impl HistoryPollState {
    /// Fixes the first-poll boundary before lookup, fetching, claims, or processing can fail.
    pub fn begin(
        &mut self,
        channel_id: u64,
        poll_started_at: PollStartedAt,
    ) -> Result<HistoryPollCycleToken, HistoryPollStateError> {
        let current = self.channels.get(&channel_id).copied();
        let state = match current {
            None => ChannelState::Priming {
                phase: HistoryPollPhase::Prime,
                boundary: poll_started_at.watermark(),
                revision: self.next_revision(channel_id)?,
            },
            Some(ChannelState::Active {
                watermark: None,
                revision: _,
            }) => ChannelState::Priming {
                phase: HistoryPollPhase::Reprime,
                boundary: poll_started_at.watermark(),
                revision: self.next_revision(channel_id)?,
            },
            Some(state) => state,
        };
        if current != Some(state) {
            self.channels.insert(channel_id, state);
        }
        Ok(HistoryPollCycleToken {
            channel_id,
            expected: expected_state(state),
        })
    }

    /// Proposes claim intent without touching `SQLite`, performing side effects, or moving state.
    ///
    /// `newest_first` is the complete fixed REST window and may contain at most ten items. The
    /// returned items are oldest-first. Commit only after every disposition finishes successfully.
    pub fn propose<T>(
        &self,
        cycle: &HistoryPollCycleToken,
        newest_first: Vec<HistoryBatchItem<T>>,
    ) -> Result<HistoryPollProposal<T>, HistoryPollStateError> {
        if newest_first.len() > HISTORY_POLL_PAGE_LIMIT {
            return Err(HistoryPollStateError::BatchTooLarge {
                actual: newest_first.len(),
            });
        }
        if self.expected_state(cycle.channel_id) != Some(cycle.expected) {
            return Err(HistoryPollStateError::StaleCycle {
                channel_id: cycle.channel_id,
            });
        }

        let latest = newest_first.iter().filter_map(|item| item.watermark).max();
        let (phase, next_watermark, mode) = proposal_parameters(cycle.expected, latest);
        let commit_token = HistoryPollCommitToken {
            channel_id: cycle.channel_id,
            expected: cycle.expected,
            next_watermark,
        };
        Ok(HistoryPollProposal::new(
            phase,
            next_watermark,
            decide_items(newest_first, mode),
            commit_token,
        ))
    }

    /// Applies a proposal after its whole batch completed. Errors never mutate state.
    pub fn commit(
        &mut self,
        channel_id: u64,
        token: &HistoryPollCommitToken,
    ) -> Result<(), HistoryPollCommitError> {
        if channel_id != token.channel_id {
            return Err(HistoryPollCommitError::ChannelMismatch {
                token_channel_id: token.channel_id,
                requested_channel_id: channel_id,
            });
        }
        if self.expected_state(channel_id) != Some(token.expected) {
            return Err(HistoryPollCommitError::Stale { channel_id });
        }
        let revision = self
            .last_revision
            .checked_add(1)
            .ok_or(HistoryPollCommitError::RevisionExhausted { channel_id })?;
        self.last_revision = revision;
        self.channels.insert(
            channel_id,
            ChannelState::Active {
                watermark: Some(token.next_watermark),
                revision,
            },
        );
        Ok(())
    }

    #[must_use]
    pub fn is_primed(&self, channel_id: u64) -> bool {
        matches!(
            self.channels.get(&channel_id),
            Some(ChannelState::Active { .. })
        )
    }

    #[must_use]
    pub fn primed_count(&self) -> usize {
        self.channels
            .values()
            .filter(|state| matches!(state, ChannelState::Active { .. }))
            .count()
    }

    /// Drops state for targets absent from the latest successfully loaded, bounded target set.
    ///
    /// Callers must not invoke this after a target-load failure and must supply at most 50 IDs.
    pub fn retain_channels(&mut self, channel_ids: &HashSet<u64>) {
        self.channels
            .retain(|channel_id, _state| channel_ids.contains(channel_id));
    }

    #[must_use]
    pub fn watermark(&self, channel_id: u64) -> Option<HistoryWatermark> {
        match self.channels.get(&channel_id) {
            Some(ChannelState::Active { watermark, .. }) => *watermark,
            None | Some(ChannelState::Priming { .. }) => None,
        }
    }

    fn expected_state(&self, channel_id: u64) -> Option<ExpectedChannelState> {
        self.channels.get(&channel_id).copied().map(expected_state)
    }

    fn next_revision(&mut self, channel_id: u64) -> Result<u64, HistoryPollStateError> {
        let revision = self
            .last_revision
            .checked_add(1)
            .ok_or(HistoryPollStateError::RevisionExhausted { channel_id })?;
        self.last_revision = revision;
        Ok(revision)
    }

    #[cfg(test)]
    fn mark_primed_without_watermark_for_test(&mut self, channel_id: u64) {
        self.last_revision = self.last_revision.max(1);
        self.channels.insert(
            channel_id,
            ChannelState::Active {
                watermark: None,
                revision: 1,
            },
        );
    }
}

fn expected_state(state: ChannelState) -> ExpectedChannelState {
    match state {
        ChannelState::Priming {
            phase,
            boundary,
            revision,
        } => ExpectedChannelState::Priming {
            phase,
            boundary,
            revision,
        },
        ChannelState::Active {
            watermark,
            revision,
        } => ExpectedChannelState::Active {
            watermark,
            revision,
        },
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
