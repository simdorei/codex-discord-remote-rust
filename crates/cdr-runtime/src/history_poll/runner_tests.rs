use super::test_support::*;
use super::*;

#[tokio::test]
async fn hpr_01_retry_reuses_prime_boundary_and_claims_candidates_once_oldest_first() {
    let mut state = HistoryPollState::default();
    let mut failed = Script::source_failure();
    assert_eq!(
        cycle(&mut state, 100, &mut failed).await,
        Err(HistoryPollRunError::Source("source"))
    );
    assert_eq!(state.watermark(CHANNEL), None);

    let mut retry = Script::with(vec![candidate(3, 150), candidate(2, 99), candidate(1, 98)]);
    retry.lost_claims.insert(2);
    let outcome = cycle(&mut state, 200, &mut retry).await.expect("retry");
    assert_eq!(
        format!("{outcome:?}"),
        "HistoryPollCycleOutcome { phase: Prime, fetched: 3, no_claim: 0, claim_attempted: 3, claim_won: 2, discarded: 2, processed: 1 }"
    );
    assert_eq!(
        events(&retry),
        "fetch:7:10,adapt:3,adapt:2,adapt:1,claim:1,claim:2,claim:3,process:3"
    );
    assert_eq!(retry.claimed_count(), 2);
    assert!(retry.was_claimed(1));
    assert!(!retry.was_claimed(2));
    assert!(retry.was_claimed(3));
    assert_eq!(state.watermark(CHANNEL), Some(key(150, 3)));
}

#[tokio::test]
async fn hpr_02_ignore_and_invalid_candidate_never_reach_claim() {
    let mut state = HistoryPollState::default();
    let mut script = Script::with(vec![ignored(1, 120), candidate(0, 121)]);
    let outcome = cycle(&mut state, 100, &mut script).await.expect("ignored");
    assert_eq!(
        format!("{outcome:?}"),
        "HistoryPollCycleOutcome { phase: Prime, fetched: 2, no_claim: 2, claim_attempted: 0, claim_won: 0, discarded: 0, processed: 0 }"
    );
    assert_eq!(events(&script), "fetch:7:10,adapt:1,adapt:0");
    assert_eq!(script.claimed_count(), 0);
    assert_eq!(state.watermark(CHANNEL), Some(key(120, 1)));
}

#[tokio::test]
async fn hpr_03_non_clone_candidate_moves_into_exactly_one_claim() {
    let mut state = HistoryPollState::default();
    let mut script = Script::with(vec![candidate(1, 101)]);
    script.lost_claims.insert(1);
    let outcome = cycle(&mut state, 100, &mut script)
        .await
        .expect("claim loss");
    assert_eq!(
        format!("{outcome:?}"),
        "HistoryPollCycleOutcome { phase: Prime, fetched: 1, no_claim: 0, claim_attempted: 1, claim_won: 0, discarded: 0, processed: 0 }"
    );
    assert_eq!(events(&script), "fetch:7:10,adapt:1,claim:1");
    assert_eq!(script.claimed_count(), 0);
    assert_eq!(state.watermark(CHANNEL), Some(key(101, 1)));
}

#[tokio::test]
async fn hpr_04_process_failure_preserves_cursor_and_stops_before_later_items() {
    let mut state = HistoryPollState::default();
    let mut script = Script::with(vec![
        candidate(3, 103),
        candidate(2, 102),
        candidate(1, 101),
    ]);
    script.process_error = Some(2);
    assert_eq!(
        cycle(&mut state, 100, &mut script).await,
        Err(HistoryPollRunError::Process("process"))
    );
    assert_eq!(script.claimed_count(), 2);
    assert_eq!(
        events(&script),
        "fetch:7:10,adapt:3,adapt:2,adapt:1,claim:1,process:1,claim:2,process:2"
    );
    assert_eq!(state.watermark(CHANNEL), None);
}

#[tokio::test]
async fn hpr_05_adaptation_and_claim_failures_preserve_cursor_and_identity() {
    let mut state = HistoryPollState::default();
    let mut adapt = Script::with(vec![
        candidate(3, 103),
        candidate(2, 102),
        candidate(1, 101),
    ]);
    adapt.adaptation_error = Some(2);
    assert_eq!(
        cycle(&mut state, 100, &mut adapt).await,
        Err(HistoryPollRunError::Adaptation("adaptation"))
    );
    assert_eq!(events(&adapt), "fetch:7:10,adapt:3,adapt:2");
    assert_eq!(adapt.claimed_count(), 0);
    assert_eq!(state.watermark(CHANNEL), None);

    let mut claim = Script::with(vec![
        candidate(3, 103),
        candidate(2, 102),
        candidate(1, 101),
    ]);
    claim.claim_error = Some(2);
    assert_eq!(
        cycle(&mut state, 200, &mut claim).await,
        Err(HistoryPollRunError::Claim("claim"))
    );
    assert_eq!(
        events(&claim),
        "fetch:7:10,adapt:3,adapt:2,adapt:1,claim:1,process:1,claim:2"
    );
    assert_eq!(claim.claimed_count(), 1);
    assert!(claim.was_claimed(1));
    assert_eq!(state.watermark(CHANNEL), None);
}

#[tokio::test]
async fn hpr_06_oversized_source_page_is_atomic() {
    let mut state = HistoryPollState::default();
    let mut script = Script::with(
        (1..=11)
            .map(|id| candidate(id, 100 + i64::from(id)))
            .collect(),
    );
    assert_eq!(
        cycle(&mut state, 100, &mut script).await,
        Err(HistoryPollRunError::State(
            HistoryPollStateError::BatchTooLarge { actual: 11 }
        ))
    );
    assert_eq!(events(&script), "fetch:7:10");
    assert_eq!(state.watermark(CHANNEL), None);
}

#[tokio::test]
async fn hpr_07_stale_commit_keeps_its_error_identity_and_cursor() {
    let mut state = StaleCommit::new();
    let mut script = Script::with(vec![candidate(1, 101)]);
    assert_eq!(
        cycle(&mut state, 100, &mut script).await,
        Err(HistoryPollRunError::Commit(HistoryPollCommitError::Stale {
            channel_id: CHANNEL
        }))
    );
    assert_eq!(events(&script), "fetch:7:10,adapt:1,claim:1,process:1");
    assert!(script.was_claimed(1));
    assert_eq!(state.watermark(), None);
}

#[tokio::test]
async fn hpr_08_cancellation_during_processing_leaves_claim_without_commit() {
    let mut state = HistoryPollState::default();
    let mut script = Script::with(vec![candidate(1, 101)]);
    script.pending_process = Some(1);
    let mut run = Box::pin(cycle(&mut state, 100, &mut script));
    tokio::select! { biased; result = &mut run => panic!("unexpected completion: {result:?}"), () = std::future::ready(()) => {} }
    drop(run);
    assert_eq!(events(&script), "fetch:7:10,adapt:1,claim:1,process:1");
    assert!(script.was_claimed(1));
    assert_eq!(script.claimed_count(), 1);
    assert_eq!(state.watermark(CHANNEL), None);
}

#[tokio::test]
async fn hpr_09_discard_and_process_claims_carry_distinct_purposes() {
    let mut state = HistoryPollState::default();
    let mut script = Script::with(vec![candidate(2, 101), candidate(1, 99)]);

    cycle(&mut state, 100, &mut script)
        .await
        .expect("mixed prime cycle");

    assert_eq!(
        script.claim_purposes,
        vec![HistoryClaimPurpose::Discard, HistoryClaimPurpose::Process]
    );
}
