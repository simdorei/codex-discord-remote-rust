use cdr_runtime::history_poll::{
    HistoryBatchItem, HistoryItemDecision, HistoryPollItem, HistoryPollState, HistoryWatermark,
    PollStartedAt,
};

fn key(micros: i64, id: u64) -> HistoryWatermark {
    HistoryWatermark::from_message(micros, id).expect("valid history key")
}

fn candidate(value: &'static str, micros: i64, id: u64) -> HistoryBatchItem<&'static str> {
    HistoryBatchItem {
        watermark: Some(key(micros, id)),
        kind: HistoryPollItem::Candidate(value),
    }
}

fn ignored(micros: i64, id: u64) -> HistoryBatchItem<&'static str> {
    HistoryBatchItem {
        watermark: Some(key(micros, id)),
        kind: HistoryPollItem::Ignore,
    }
}

#[test]
fn equal_timestamp_uses_the_integer_id_and_equal_key_is_discarded() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    let prime = state.propose(&cycle, vec![ignored(100, 5)]).expect("prime");
    state.commit(7, prime.commit_token()).expect("commit prime");
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(200))
        .expect("begin incremental");
    let proposal = state
        .propose(
            &cycle,
            vec![candidate("higher", 100, 6), candidate("equal", 100, 5)],
        )
        .expect("incremental");

    assert_eq!(
        proposal.items_oldest_first[0],
        HistoryItemDecision::ClaimAndDiscard("equal")
    );
    assert_eq!(
        proposal.items_oldest_first[1],
        HistoryItemDecision::ClaimAndProcess("higher")
    );
}

#[test]
fn extreme_timestamp_and_message_ids_keep_lexicographic_order_without_arithmetic() {
    assert!(key(i64::MIN, u64::MAX) < key(i64::MAX, 1));
    assert!(key(0, u64::MAX - 1) < key(0, u64::MAX));
}

#[test]
fn normal_poll_discards_old_candidates_and_never_claims_ignored_messages() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    let prime = state.propose(&cycle, vec![ignored(100, 5)]).expect("prime");
    state.commit(7, prime.commit_token()).expect("commit prime");
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(200))
        .expect("begin incremental");
    let proposal = state
        .propose(
            &cycle,
            vec![
                ignored(102, 7),
                candidate("candidate-new", 101, 6),
                candidate("candidate-old", 100, 5),
            ],
        )
        .expect("incremental proposal");

    assert_eq!(
        proposal.items_oldest_first,
        [
            HistoryItemDecision::ClaimAndDiscard("candidate-old"),
            HistoryItemDecision::ClaimAndProcess("candidate-new"),
            HistoryItemDecision::NoClaim,
        ]
    );
    assert_eq!(proposal.next_watermark, key(102, 7));
}
