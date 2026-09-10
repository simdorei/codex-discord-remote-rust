use super::*;

#[tokio::test]
async fn two_explicit_cycles_prove_repetition_independently_of_machine_speed() {
    let temp = tempfile::tempdir().unwrap();
    let mut harness = OfflineHarness::create(temp.path(), 424_242).unwrap();
    let mut counters = SoakCounters::default();
    harness.run_recovery_probe(&mut counters).await.unwrap();
    for cycle in 1..=2 {
        harness
            .run_cycle(cycle, 424_242, &mut counters)
            .await
            .unwrap();
        assert_eq!(counters.queue_remaining, 0);
        assert_eq!(counters.outbox_remaining, 0);
        assert_eq!(counters.queue_submitted, cycle * 4 + 2);
        assert_eq!(counters.queue_submitted, counters.queue_completed);
        assert_eq!(counters.outbox_staged, counters.outbox_delivered);
        assert_eq!(counters.mirror_sent, cycle);
        assert_eq!(counters.mirror_events, cycle * 3);
        assert_eq!(counters.duplicate_successes, 0);
        assert_eq!(counters.target_stalls, 0);
    }
    assert_eq!(counters.mirror_send_failures, 1);
    assert_eq!(counters.mirror_send_retries, 1);
    assert_eq!(counters.mirror_retry_same_message, 1);
    assert_eq!(counters.schedule_steps, 2);
    assert_eq!(
        harness.schedule_digest(),
        "29e79edee9130f9edc7a8a371031cb58053866e2ba757c065b3b4b494ac39b80"
    );
}
