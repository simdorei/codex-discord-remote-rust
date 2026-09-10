use std::sync::{Arc, Mutex};
use std::time::Duration;

use cdr_discord::delivery::{DeliveryFailure, DeliveryPolicy, deliver_text};

#[tokio::test]
async fn default_policy_matches_python_retry_and_marker_contract() {
    let policy = DeliveryPolicy::default();
    assert_eq!(
        policy.retry_delays,
        vec![Duration::from_millis(750), Duration::from_secs(2)]
    );
    assert!(policy.chunk_markers);
}

#[tokio::test]
async fn retries_only_the_failed_chunk_and_reports_sent_count() {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&attempts);
    let mut failures = 1;
    let sent = deliver_text(
        &"x".repeat(2_100),
        &DeliveryPolicy {
            retry_delays: vec![Duration::ZERO],
            chunk_markers: true,
        },
        move |chunk| {
            recorded.lock().unwrap().push(chunk);
            let fail = failures > 0;
            failures -= usize::from(fail);
            async move { if fail { Err("temporary") } else { Ok(()) } }
        },
    )
    .await
    .unwrap();

    assert_eq!(sent, 2);
    let attempts = attempts.lock().unwrap();
    assert_eq!(attempts.len(), 3);
    assert_eq!(attempts[0], attempts[1]);
    assert_ne!(attempts[1], attempts[2]);
}

#[tokio::test]
async fn final_failure_exposes_part_attempts_and_source() {
    let error = deliver_text(
        "hello",
        &DeliveryPolicy {
            retry_delays: vec![Duration::ZERO, Duration::ZERO],
            chunk_markers: true,
        },
        |_| async { Err::<(), _>("Discord unavailable") },
    )
    .await
    .unwrap_err();

    assert_eq!(
        error,
        DeliveryFailure {
            part: 1,
            total_parts: 1,
            attempts: 3,
            source: "Discord unavailable"
        }
    );
}
