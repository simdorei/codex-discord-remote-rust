use super::*;
use crate::history_poll::{HistoryItemDecision, HistoryPollItem};

fn key(micros: i64, id: u64) -> HistoryWatermark {
    HistoryWatermark::from_message(micros, id).expect("valid message key")
}

fn candidate(
    value: &'static str,
    watermark: Option<HistoryWatermark>,
) -> HistoryBatchItem<&'static str> {
    HistoryBatchItem {
        watermark,
        kind: HistoryPollItem::Candidate(value),
    }
}

fn ignored(watermark: Option<HistoryWatermark>) -> HistoryBatchItem<&'static str> {
    HistoryBatchItem {
        watermark,
        kind: HistoryPollItem::Ignore,
    }
}

#[test]
fn missing_watermark_reprimes_and_only_claims_valid_candidates_for_discard() {
    let mut state = HistoryPollState::default();
    state.mark_primed_without_watermark_for_test(44);
    let cycle = state
        .begin(44, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin reprime");
    let proposal = state
        .propose(
            &cycle,
            vec![
                ignored(Some(key(102, 2))),
                candidate("candidate", Some(key(101, 1))),
                candidate("invalid", None),
            ],
        )
        .expect("reprime");

    assert_eq!(proposal.phase, HistoryPollPhase::Reprime);
    assert_eq!(
        proposal.items_oldest_first,
        [
            HistoryItemDecision::NoClaim,
            HistoryItemDecision::ClaimAndDiscard("candidate"),
            HistoryItemDecision::NoClaim,
        ]
    );
    assert_eq!(proposal.next_watermark, key(102, 2));
    assert_eq!(state.watermark(44), None);
    state.commit(44, proposal.commit_token()).expect("commit");
    assert_eq!(state.watermark(44), Some(key(102, 2)));
}

#[test]
fn oversized_first_proposal_does_not_publish_a_primed_watermark() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(44, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    let eleven = (0..=HISTORY_POLL_PAGE_LIMIT)
        .map(|_| candidate("candidate", None))
        .collect();

    assert_eq!(
        state.propose(&cycle, eleven),
        Err(HistoryPollStateError::BatchTooLarge { actual: 11 })
    );
    assert!(!state.is_primed(44));
    assert_eq!(state.watermark(44), None);
}

#[test]
fn ignored_only_incremental_poll_advances_without_requesting_a_claim() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(44, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    let prime = state
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("prime");
    state
        .commit(44, prime.commit_token())
        .expect("commit prime");

    let cycle = state
        .begin(44, PollStartedAt::from_normalized_utc_micros(999))
        .expect("begin incremental");
    let proposal = state
        .propose(&cycle, vec![ignored(Some(key(101, 1)))])
        .expect("incremental");

    assert_eq!(proposal.next_watermark, key(101, 1));
    assert_eq!(proposal.items_oldest_first[0], HistoryItemDecision::NoClaim);
    state
        .commit(44, proposal.commit_token())
        .expect("commit ignored-only advance");
    assert_eq!(state.watermark(44), Some(key(101, 1)));
}

#[test]
fn revision_exhaustion_leaves_begin_and_commit_state_unchanged() {
    let mut begin_state = HistoryPollState {
        last_revision: u64::MAX,
        ..HistoryPollState::default()
    };
    let before_begin = format!("{begin_state:?}");
    assert_eq!(
        begin_state.begin(44, PollStartedAt::from_normalized_utc_micros(100)),
        Err(HistoryPollStateError::RevisionExhausted { channel_id: 44 })
    );
    assert_eq!(format!("{begin_state:?}"), before_begin);

    let mut commit_state = HistoryPollState::default();
    let cycle = commit_state
        .begin(44, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin before exhaustion");
    let proposal = commit_state
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("proposal before exhaustion");
    commit_state.last_revision = u64::MAX;
    let before_commit = format!("{commit_state:?}");
    assert_eq!(
        commit_state.commit(44, proposal.commit_token()),
        Err(HistoryPollCommitError::RevisionExhausted { channel_id: 44 })
    );
    assert_eq!(format!("{commit_state:?}"), before_commit);
}
