use cdr_runtime::history_poll::{
    HISTORY_POLL_PAGE_LIMIT, HistoryBatchItem, HistoryItemDecision, HistoryPollItem,
    HistoryPollPhase, HistoryPollState, HistoryPollStateError, HistoryWatermark, PollStartedAt,
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
fn newest_first_input_returns_plan_aware_claim_decisions_oldest_first() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    let proposal = state
        .propose(
            &cycle,
            vec![
                ignored(103, 3),
                candidate("new", 102, 2),
                candidate("old", 99, 1),
            ],
        )
        .expect("prime decision");

    assert_eq!(
        proposal.items_oldest_first,
        [
            HistoryItemDecision::ClaimAndDiscard("old"),
            HistoryItemDecision::ClaimAndProcess("new"),
            HistoryItemDecision::NoClaim,
        ]
    );
    assert_eq!(proposal.next_watermark, key(103, 3));
}

#[test]
fn empty_prime_and_a_new_instance_both_establish_a_fresh_boundary() {
    let mut retained = HistoryPollState::default();
    let cycle = retained
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    let first = retained
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("empty prime");
    assert_eq!(first.phase, HistoryPollPhase::Prime);
    assert!(first.items_oldest_first.is_empty());
    retained
        .commit(7, first.commit_token())
        .expect("commit prime");
    let cycle = retained
        .begin(7, PollStartedAt::from_normalized_utc_micros(200))
        .expect("begin incremental");
    let repeated_ready = retained
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("retained state");
    assert_eq!(repeated_ready.phase, HistoryPollPhase::Incremental);

    let mut restarted_state = HistoryPollState::default();
    let cycle = restarted_state
        .begin(7, PollStartedAt::from_normalized_utc_micros(200))
        .expect("begin after restart");
    let restarted = restarted_state
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("new process state");
    assert_eq!(restarted.phase, HistoryPollPhase::Prime);
}

#[test]
fn channels_keep_independent_priming_and_watermarks() {
    let mut state = HistoryPollState::default();
    let cycle = state
        .begin(1, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin channel one");
    let prime = state
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("channel one prime");
    state.commit(1, prime.commit_token()).expect("commit one");
    let cycle = state
        .begin(2, PollStartedAt::from_normalized_utc_micros(200))
        .expect("begin channel two");
    let prime = state
        .propose(&cycle, Vec::<HistoryBatchItem<()>>::new())
        .expect("channel two prime");
    state.commit(2, prime.commit_token()).expect("commit two");

    let cycle = state
        .begin(1, PollStartedAt::from_normalized_utc_micros(999))
        .expect("begin channel one incremental");
    let one = state
        .propose(&cycle, vec![candidate("one", 150, 1)])
        .expect("channel one incremental");
    let cycle = state
        .begin(2, PollStartedAt::from_normalized_utc_micros(999))
        .expect("begin channel two incremental");
    let two = state
        .propose(&cycle, vec![candidate("two", 150, 2)])
        .expect("channel two incremental");

    assert_eq!(
        one.items_oldest_first[0],
        HistoryItemDecision::ClaimAndProcess("one")
    );
    assert_eq!(
        two.items_oldest_first[0],
        HistoryItemDecision::ClaimAndDiscard("two")
    );
    state
        .commit(1, one.commit_token())
        .expect("commit one update");
    assert_eq!(state.watermark(1), Some(key(150, 1)));
    assert_ne!(state.watermark(1), state.watermark(2));
}

#[test]
fn invalid_id_is_ineligible_and_ten_item_window_is_transactional() {
    assert_eq!(HistoryWatermark::from_message(100, 0), None);
    let mut state = HistoryPollState::default();
    let ten = (0..HISTORY_POLL_PAGE_LIMIT)
        .map(|_| HistoryBatchItem {
            watermark: None,
            kind: HistoryPollItem::<()>::Ignore,
        })
        .collect();
    let cycle = state
        .begin(7, PollStartedAt::from_normalized_utc_micros(100))
        .expect("begin prime");
    state.propose(&cycle, ten).expect("ten-item fixed window");
    let before = state.watermark(7);
    let eleven = (0..=HISTORY_POLL_PAGE_LIMIT)
        .map(|_| HistoryBatchItem {
            watermark: None,
            kind: HistoryPollItem::Candidate(()),
        })
        .collect();

    assert_eq!(
        state.propose(&cycle, eleven),
        Err(HistoryPollStateError::BatchTooLarge { actual: 11 })
    );
    assert_eq!(state.watermark(7), before);
}
