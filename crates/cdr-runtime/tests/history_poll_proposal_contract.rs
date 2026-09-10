use cdr_runtime::history_poll::{
    HistoryBatchItem, HistoryItemDecision, HistoryPollItem, HistoryPollPhase, HistoryPollState,
    HistoryWatermark, PollStartedAt,
};

fn key(micros: i64, id: u64) -> HistoryWatermark {
    HistoryWatermark::from_message(micros, id).expect("valid key")
}

#[test]
fn consuming_a_proposal_moves_non_clone_payloads_before_committing() {
    struct NonClone(&'static str);

    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin");
    let proposal = state
        .propose(
            &cycle,
            vec![HistoryBatchItem {
                watermark: Some(key(101, 1)),
                kind: HistoryPollItem::Candidate(NonClone("owned")),
            }],
        )
        .expect("proposal");

    let (phase, next_watermark, items, commit_token) = proposal.into_parts();
    assert_eq!(phase, HistoryPollPhase::Prime);
    assert_eq!(next_watermark, key(101, 1));
    let item = items.into_iter().next().expect("owned item");
    let HistoryItemDecision::ClaimAndProcess(item) = item else {
        panic!("new candidate must carry its payload into processing");
    };
    assert_eq!(item.0, "owned");
    state
        .commit(7, &commit_token)
        .expect("commit after moving items");
}

#[test]
fn empty_incremental_page_preserves_the_active_watermark() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    let prime = state
        .propose(
            &cycle,
            vec![HistoryBatchItem {
                watermark: Some(key(110, 1)),
                kind: HistoryPollItem::<()>::Ignore,
            }],
        )
        .expect("prime proposal");
    state.commit(7, prime.commit_token()).expect("commit prime");

    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(999))
        .expect("begin incremental");
    let empty = state
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("empty incremental proposal");
    assert_eq!(empty.next_watermark, key(110, 1));
    assert!(empty.items_oldest_first.is_empty());
    state
        .commit(7, empty.commit_token())
        .expect("commit empty page");
    assert_eq!(state.watermark(7), Some(key(110, 1)));
}
