use cdr_store::mirror::{claim_event, get_offset, get_or_init_cursor, has_event, update_cursor};
use cdr_store::processed::{claim, is_processed, mark};

#[test]
fn m1_processed_messages_are_permanent_and_mirror_events_are_idempotent_across_reopen() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("mirror.sqlite");

    assert!(claim(&path, 42, 100.0).expect("claim message"));
    assert!(!claim(&path, 42, 101.0).expect("duplicate message"));
    assert!(is_processed(&path, 42).expect("message survives reopen"));
    mark(&path, 42, 150.0).expect("refresh processed time");
    assert!(!claim(&path, 42, 10_000_000_000.0).expect("reject far-future duplicate"));
    assert!(is_processed(&path, 42).expect("message remains a permanent replay barrier"));

    assert!(claim_event(&path, "digest-1", "thread-1", 200.0).expect("claim event"));
    assert!(!claim_event(&path, "digest-1", "thread-1", 201.0).expect("duplicate event"));
    assert!(has_event(&path, "digest-1", "thread-1").expect("event survives reopen"));
    assert!(!has_event(&path, "digest-1", "thread-other").expect("thread scope checked"));
}

#[test]
fn m2_cursor_resumes_same_rollout_and_resets_for_changed_rollout() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("mirror.sqlite");

    assert_eq!(
        get_or_init_cursor(&path, "thread-1", "rollout-a.jsonl", 10, 100.0)
            .expect("initialize cursor"),
        10
    );
    update_cursor(&path, "thread-1", "rollout-a.jsonl", 25, 110.0).expect("update cursor");
    assert_eq!(
        get_or_init_cursor(&path, "thread-1", "rollout-a.jsonl", 999, 120.0)
            .expect("resume same rollout"),
        25
    );
    assert_eq!(
        get_or_init_cursor(&path, "thread-1", "rollout-b.jsonl", 5, 130.0)
            .expect("reset changed rollout"),
        5
    );
    let offset = get_offset(&path, "thread-1")
        .expect("get offset")
        .expect("offset exists");
    assert_eq!(offset.rollout_path, "rollout-b.jsonl");
    assert_eq!(offset.cursor, 5);
    assert!((offset.updated_at - 130.0).abs() < f64::EPSILON);
}
