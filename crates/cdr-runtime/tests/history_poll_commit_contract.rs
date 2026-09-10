use std::collections::HashSet;

use cdr_runtime::history_poll::{
    HistoryBatchItem, HistoryItemDecision, HistoryPollCommitError, HistoryPollItem,
    HistoryPollPhase, HistoryPollState, HistoryPollStateError, HistoryWatermark, PollStartedAt,
};

fn key(micros: i64, id: u64) -> HistoryWatermark {
    HistoryWatermark::from_message(micros, id).expect("valid key")
}

fn candidate(value: &'static str, micros: i64, id: u64) -> HistoryBatchItem<&'static str> {
    HistoryBatchItem {
        watermark: Some(key(micros, id)),
        kind: HistoryPollItem::Candidate(value),
    }
}

#[test]
fn failed_first_cycle_reuses_its_fixed_boundary_and_does_not_skip_an_intermediate_message() {
    let mut state = HistoryPollState::default();
    let first_cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin at t0");
    let failed_proposal = state
        .propose(&first_cycle, vec![candidate("first", 101, 1)])
        .expect("side-effect-free proposal");
    assert_eq!(failed_proposal.phase, HistoryPollPhase::Prime);
    assert!(!state.is_primed(7));
    assert_eq!(state.watermark(7), None);
    drop(failed_proposal);

    let retry_cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(200))
        .expect("retry at t1");
    let retry = state
        .propose(&retry_cycle, vec![candidate("between", 150, 2)])
        .expect("retry proposal");

    assert_eq!(retry.phase, HistoryPollPhase::Prime);
    assert_eq!(
        retry.items_oldest_first[0],
        HistoryItemDecision::ClaimAndProcess("between")
    );
}

#[test]
fn commit_once_publishes_the_cursor_and_double_commit_is_rejected_without_mutation() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin");
    let proposal = state
        .propose(&cycle, vec![candidate("message", 101, 1)])
        .expect("proposal");

    state
        .commit(7, proposal.commit_token())
        .expect("first commit");
    assert!(state.is_primed(7));
    assert_eq!(state.primed_count(), 1);
    assert_eq!(state.watermark(7), Some(key(101, 1)));
    let committed_state = format!("{state:?}");
    assert_eq!(
        state.commit(7, proposal.commit_token()),
        Err(HistoryPollCommitError::Stale { channel_id: 7 })
    );
    assert_eq!(format!("{state:?}"), committed_state);
    assert_eq!(state.watermark(7), Some(key(101, 1)));
}

#[test]
fn competing_proposal_becomes_stale_and_cannot_overwrite_the_committed_cursor() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    let prime = state
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("prime");
    state.commit(7, prime.commit_token()).expect("commit prime");

    let first_cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(200))
        .expect("first incremental cycle");
    let second_cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(200))
        .expect("competing incremental cycle");
    let first = state
        .propose(&first_cycle, vec![candidate("first", 101, 1)])
        .expect("first proposal");
    let competing = state
        .propose(&second_cycle, vec![candidate("stale", 102, 2)])
        .expect("competing proposal");
    state.commit(7, first.commit_token()).expect("commit first");

    let committed_state = format!("{state:?}");
    assert_eq!(
        state.commit(7, competing.commit_token()),
        Err(HistoryPollCommitError::Stale { channel_id: 7 })
    );
    assert_eq!(format!("{state:?}"), committed_state);
    assert_eq!(state.watermark(7), Some(key(101, 1)));
}

#[test]
fn cross_channel_commit_is_rejected_without_mutating_either_channel() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(1, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin one");
    let proposal = state
        .propose(&cycle, vec![candidate("one", 101, 1)])
        .expect("proposal one");
    state
        .begin(2, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin two");

    let pending_state = format!("{state:?}");
    assert_eq!(
        state.commit(2, proposal.commit_token()),
        Err(HistoryPollCommitError::ChannelMismatch {
            token_channel_id: 1,
            requested_channel_id: 2,
        })
    );
    assert_eq!(format!("{state:?}"), pending_state);
    assert_eq!(state.primed_count(), 0);
    assert_eq!(state.watermark(1), None);
    assert_eq!(state.watermark(2), None);
}

#[test]
fn retaining_successful_targets_prunes_churn_and_reentry_primes_again() {
    let mut state = HistoryPollState::default();
    for channel_id in [1, 2] {
        let cycle = state
            .begin(channel_id, PollStartedAt::from_normalized_utc_micros(100))
            .expect("begin");
        let proposal = state
            .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
            .expect("proposal");
        state
            .commit(channel_id, proposal.commit_token())
            .expect("commit");
    }
    state
        .begin(3, PollStartedAt::from_normalized_utc_micros(100))
        .expect("pending target");

    state.retain_channels(&HashSet::from([2]));
    assert_eq!(state.primed_count(), 1);
    assert!(!state.is_primed(1));
    let reprime_pending = state
        .begin(3, PollStartedAt::from_normalized_utc_micros(200))
        .expect("reenter pending target");
    let pending = state
        .propose(&reprime_pending, vec![candidate("old", 150, 1)])
        .expect("new pending boundary");
    assert_eq!(pending.phase, HistoryPollPhase::Prime);
    assert_eq!(
        pending.items_oldest_first[0],
        HistoryItemDecision::ClaimAndDiscard("old")
    );
    let reentered = state
        .begin(1, PollStartedAt::from_normalized_utc_micros(200))
        .expect("reentered target");
    assert_eq!(
        state
            .propose(&reentered, Vec::<HistoryBatchItem<()>>::new())
            .expect("reentry proposal")
            .phase,
        HistoryPollPhase::Prime
    );
}

#[test]
fn pruning_and_readding_a_channel_cannot_revive_an_old_commit_token() {
    let mut state = HistoryPollState::default();
    let old_cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("old begin");
    let old = state
        .propose(&old_cycle, vec![candidate("old", 101, 1)])
        .expect("old proposal");
    state.retain_channels(&HashSet::new());
    let new_cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("new begin with the same boundary");
    let before_stale_proposal = format!("{state:?}");
    assert_eq!(
        state.propose(&old_cycle, Vec::<HistoryBatchItem<()>>::new()),
        Err(HistoryPollStateError::StaleCycle { channel_id: 7 })
    );
    assert_eq!(format!("{state:?}"), before_stale_proposal);
    let new = state
        .propose(&new_cycle, vec![candidate("new", 102, 2)])
        .expect("new proposal");

    assert_eq!(
        state.commit(7, old.commit_token()),
        Err(HistoryPollCommitError::Stale { channel_id: 7 })
    );
    assert_eq!(state.watermark(7), None);
    state.commit(7, new.commit_token()).expect("new commit");
    assert_eq!(state.watermark(7), Some(key(102, 2)));
}
