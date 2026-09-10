use std::time::Duration;

use super::supervisor::RestartBackoff;

// SUP-C3: failed restarts use the exact bounded schedule and success resets it.
#[test]
fn restart_backoff_is_bounded_and_resets_after_success() {
    let mut backoff = RestartBackoff::default();
    let actual = (0..7).map(|_| backoff.next_delay()).collect::<Vec<_>>();

    assert_eq!(
        actual,
        [250, 500, 1_000, 2_000, 4_000, 5_000, 5_000].map(Duration::from_millis)
    );

    backoff.reset();
    assert_eq!(backoff.next_delay(), Duration::from_millis(250));
}
