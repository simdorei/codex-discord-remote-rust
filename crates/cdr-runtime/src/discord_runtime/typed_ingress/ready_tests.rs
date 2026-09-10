use std::time::Duration;

use super::retry_delay;

#[test]
fn tir_01_each_setup_loop_has_the_same_bounded_retry_schedule() {
    let expected = [1, 2, 4, 8, 16, 30, 30, 30].map(Duration::from_secs);
    let actual = std::array::from_fn(retry_delay);

    assert_eq!(actual, expected);
}

#[test]
fn tir_02_one_setup_retry_index_cannot_advance_the_other() {
    let registration_failures = 8;
    let notice_failures = 0;

    assert_eq!(retry_delay(registration_failures), Duration::from_secs(30));
    assert_eq!(retry_delay(notice_failures), Duration::from_secs(1));
}
