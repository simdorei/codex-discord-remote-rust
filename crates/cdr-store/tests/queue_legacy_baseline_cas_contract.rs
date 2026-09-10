use std::sync::{Arc, Barrier};

use cdr_store::delivery::{complete as complete_delivery, list_pending};
use cdr_store::queue::{QueueJobState, STARTING_CANDIDATE_HOLD_PREFIX, list};

#[path = "support/legacy_baseline.rs"]
mod legacy_baseline;
use legacy_baseline::{
    COMPACT_JSON, ESCAPED_JSON, Fixture, OPERATIONS, Operation, PYTHON_JSON, SPACED_JSON, apply,
    baseline_values, replace_baseline,
};

#[test]
fn python_baseline_can_mark_running_without_rewriting_saved_bytes() {
    let fixture = Fixture::new(Operation::Running, PYTHON_JSON);
    let running = fixture
        .apply(Operation::Running)
        .unwrap()
        .expect("the parsed legacy Starting snapshot must win its running CAS");
    assert_eq!(running.state, QueueJobState::Running);
    assert_eq!(running.turn_id.as_deref(), Some("observed-turn"));
    assert_eq!(running.baseline_turn_ids, baseline_values());
    assert_eq!(fixture.raw_baseline(), PYTHON_JSON);
    assert!(fixture.apply(Operation::Running).unwrap().is_none());
}

#[test]
fn python_baseline_can_record_definite_and_ambiguous_start_failures() {
    for (operation, expected) in [
        (Operation::DefiniteFailure, QueueJobState::Pending),
        (Operation::AmbiguousFailure, QueueJobState::Starting),
    ] {
        let fixture = Fixture::new(operation, PYTHON_JSON);
        let updated = fixture
            .apply(operation)
            .unwrap()
            .expect("legacy formatting must not suppress the observed failure");
        assert_eq!(updated.state, expected);
        assert_eq!(updated.last_error, "observed start failure");
        assert_eq!(updated.baseline_turn_ids, baseline_values());
        assert_eq!(fixture.raw_baseline(), PYTHON_JSON);
        assert!(fixture.apply(operation).unwrap().is_none());
    }
}

#[test]
fn python_baseline_can_create_one_visible_ambiguous_hold() {
    let fixture = Fixture::new(Operation::Hold, PYTHON_JSON);
    let held = fixture
        .apply(Operation::Hold)
        .unwrap()
        .expect("the parsed legacy Starting snapshot must persist its hold and notice");
    assert_eq!(held.state, QueueJobState::Starting);
    assert!(held.last_error.starts_with(STARTING_CANDIDATE_HOLD_PREFIX));
    let notices = list_pending(&fixture.path).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.contains("candidate-after-a"));
    assert_eq!(fixture.raw_baseline(), PYTHON_JSON);
    assert!(fixture.apply(Operation::Hold).unwrap().is_none());
}

#[test]
fn python_baseline_can_refresh_only_the_pending_notice_without_recreating_delivery() {
    let fixture = Fixture::new(Operation::Refresh, PYTHON_JSON);
    let refreshed = fixture
        .apply(Operation::Refresh)
        .unwrap()
        .expect("legacy formatting must not hide the held snapshot from notice refresh");
    assert_eq!(refreshed, fixture.claimed);
    assert_eq!(fixture.raw_baseline(), PYTHON_JSON);
    let notices = list_pending(&fixture.path).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.contains("candidate-after-b"));
    assert!(!notices[0].content.contains("candidate-before"));
    assert!(complete_delivery(&fixture.path, &notices[0].delivery_id).unwrap());
    assert!(fixture.apply(Operation::Refresh).unwrap().is_some());
    assert!(list_pending(&fixture.path).unwrap().is_empty());
}

#[test]
fn equivalent_whitespace_and_unicode_or_slash_escapes_match_for_every_cas_path() {
    for operation in OPERATIONS {
        for encoding in [SPACED_JSON, ESCAPED_JSON] {
            let fixture = Fixture::new(operation, encoding);
            assert!(
                fixture.apply(operation).unwrap().is_some(),
                "{operation:?}: {encoding}"
            );
            assert_eq!(fixture.raw_baseline(), encoding);
        }
    }
}

#[test]
fn changed_order_content_multiplicity_or_json_types_cannot_win_any_cas_path() {
    let changed = [
        r#"["path/one","baseline-a","기준","🚀","baseline-a"]"#,
        r#"["baseline-a","changed","기준","🚀","baseline-a"]"#,
        r#"["baseline-a","path/one","기준","🚀"]"#,
        r#"["baseline-a","path/one","기준","🚀",1]"#,
        r#"{"0":"baseline-a"}"#,
        "null",
    ];
    for operation in OPERATIONS {
        for encoding in changed {
            let fixture = Fixture::new(operation, COMPACT_JSON);
            replace_baseline(&fixture.path, encoding);
            let before = list(&fixture.path).unwrap();
            let notices = list_pending(&fixture.path).unwrap();
            assert!(
                fixture.apply(operation).unwrap().is_none(),
                "{operation:?}: {encoding}"
            );
            assert_eq!(fixture.raw_baseline(), encoding);
            assert_eq!(list(&fixture.path).unwrap(), before);
            assert_eq!(list_pending(&fixture.path).unwrap(), notices);
        }
    }
}

#[test]
fn malformed_current_baseline_surfaces_the_parse_error_without_mutation() {
    for operation in OPERATIONS {
        let fixture = Fixture::new(operation, COMPACT_JSON);
        let before = list(&fixture.path).unwrap();
        let notices = list_pending(&fixture.path).unwrap();
        replace_baseline(&fixture.path, "[");
        assert!(fixture.apply(operation).is_err(), "{operation:?}");
        assert_eq!(fixture.raw_baseline(), "[");
        replace_baseline(&fixture.path, COMPACT_JSON);
        assert_eq!(list(&fixture.path).unwrap(), before);
        assert_eq!(list_pending(&fixture.path).unwrap(), notices);
    }
}

#[test]
fn legacy_baseline_does_not_weaken_other_snapshot_fences() {
    for operation in OPERATIONS {
        for field in 0..4 {
            let mut fixture = Fixture::new(operation, PYTHON_JSON);
            match field {
                0 => fixture.claimed.target_thread_id = "wrong-target".into(),
                1 => fixture.claimed.app_server_generation += 1,
                2 => fixture.claimed.attempt_count += 1,
                _ => fixture.claimed.updated_at += 1.0,
            }
            let before = list(&fixture.path).unwrap();
            let notices = list_pending(&fixture.path).unwrap();
            assert!(
                fixture.apply(operation).unwrap().is_none(),
                "{operation:?}: fence {field}"
            );
            assert_eq!(list(&fixture.path).unwrap(), before);
            assert_eq!(list_pending(&fixture.path).unwrap(), notices);
            assert_eq!(fixture.raw_baseline(), PYTHON_JSON);
        }
    }
}

#[test]
fn legacy_baseline_still_allows_exactly_one_concurrent_transition_winner() {
    let fixture = Fixture::new(Operation::Hold, PYTHON_JSON);
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = [
        Operation::Running,
        Operation::DefiniteFailure,
        Operation::Hold,
    ]
    .into_iter()
    .map(|operation| {
        let path = fixture.path.clone();
        let claimed = fixture.claimed.clone();
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            apply(&path, &claimed, operation).unwrap()
        })
    })
    .collect();
    let winners: Vec<_> = handles
        .into_iter()
        .filter_map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(list(&fixture.path).unwrap(), winners);
    assert_eq!(fixture.raw_baseline(), PYTHON_JSON);
    let expected_notices = usize::from(
        winners[0]
            .last_error
            .starts_with(STARTING_CANDIDATE_HOLD_PREFIX),
    );
    assert_eq!(list_pending(&fixture.path).unwrap().len(), expected_notices);
}
