use cdr_store::{claims, control_binding};
use std::path::Path;

fn choice(path: &Path, now: f64) -> String {
    claims::create_busy_choice(
        path,
        claims::NewBusyChoice {
            owner_user_id: 1,
            channel_id: 2,
            target_thread_id: Some("original"),
            prompt: "direction",
            allow_steer: true,
            now,
            time_to_live: 100.0,
        },
    )
    .unwrap()
}

#[test]
fn unrelated_choice_creation_preserves_inflight_claim_and_original_binding() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    let original = choice(&path, 1.0);
    control_binding::bind(&path, &original, "original", Some("first"), None).unwrap();
    assert!(claims::claim_busy_choice(&path, &original, 2.0).unwrap());
    let unrelated = choice(&path, 3.0);
    control_binding::bind(&path, &unrelated, "other", Some("other-turn"), None).unwrap();
    assert!(
        claims::release_busy_choice_claim(&path, &original).unwrap(),
        "a proven pre-dispatch failure must still be able to release its claim"
    );
    assert!(
        claims::get_busy_choice(&path, &original, 4.0)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        control_binding::resolve(&path, &original, "original")
            .unwrap()
            .as_deref(),
        Some("first")
    );
}

#[test]
fn reading_claimed_choice_does_not_destroy_it_before_safe_release() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    let original = choice(&path, 1.0);
    assert!(claims::claim_busy_choice(&path, &original, 2.0).unwrap());
    assert!(
        claims::get_busy_choice(&path, &original, 3.0)
            .unwrap()
            .is_none()
    );
    assert!(claims::release_busy_choice_claim(&path, &original).unwrap());
    assert!(
        claims::get_busy_choice(&path, &original, 4.0)
            .unwrap()
            .is_some()
    );
}

#[test]
fn retained_claim_never_becomes_available_without_release() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    let original = choice(&path, 1.0);
    assert!(claims::claim_busy_choice(&path, &original, 2.0).unwrap());
    claims::cleanup_busy_choices(&path, 3.0).unwrap();
    assert!(!claims::claim_busy_choice(&path, &original, 4.0).unwrap());
    assert!(
        claims::get_busy_choice(&path, &original, 4.0)
            .unwrap()
            .is_none()
    );
    assert!(
        claims::get_busy_choice(&path, &original, 102.0)
            .unwrap()
            .is_none()
    );
    assert!(!claims::claim_busy_choice(&path, &original, 102.0).unwrap());
}
