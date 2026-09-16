//! Store side of revision 11's fresh-revalidation inheritance contract.
use cdr_store::reserve_policy::{self as policy, EpisodeIdentity};

fn confirmed(database: &std::path::Path) -> policy::Policy {
    let initial = policy::ensure(database, "thread-b").unwrap();
    let claim = policy::begin_episode_claim(
        database,
        "thread-b",
        initial.revision,
        EpisodeIdentity {
            account_id: "account-a",
            process_id: Some(10),
            generation: 1,
        },
        ("model-a", Some("high"), Some("priority")),
        true,
        (Some("gpt-reserve"), Some("high"), Some("default")),
    )
    .unwrap()
    .unwrap();
    assert!(
        policy::finish_episode_claim(database, "thread-b", claim, "entering", "reserve").unwrap()
    );
    policy::get(database, "thread-b").unwrap().unwrap()
}

#[test]
fn revalidated_restore_accepts_a_new_resident_but_preserves_exact_claim_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("fixture.sqlite");
    let episode = confirmed(&db);
    policy::stage_usage_failure(&db, "thread-b", "earlier failure").unwrap();
    assert!(
        policy::begin_restore_claim(&db, "thread-b", episode.revision, "account-b", Some(20), 2)
            .unwrap()
            .is_none()
    );
    // The runtime must have freshly validated exact live settings before this API.
    let restore =
        policy::begin_restore_claim(&db, "thread-b", episode.revision, "account-a", Some(20), 2)
            .unwrap()
            .unwrap();
    let starting = policy::get(&db, "thread-b").unwrap().unwrap();
    assert_eq!(starting.process_id, Some(20));
    assert_eq!(starting.generation, Some(2));
    assert_eq!(starting.previous_model, episode.previous_model);
    assert_eq!(starting.previous_effort, episode.previous_effort);
    assert_eq!(starting.previous_tier, episode.previous_tier);
    assert!(
        policy::begin_restore_claim(&db, "thread-b", episode.revision, "account-a", Some(10), 1)
            .unwrap()
            .is_none()
    );
    let captured = restore.usage_failure.unwrap();
    policy::stage_usage_failure(&db, "thread-b", "later failure").unwrap();
    assert!(
        policy::finish_episode_claim(&db, "thread-b", restore, "restoring", "ordinary").unwrap()
    );
    let later = policy::usage_failure_claim(&db, "thread-b")
        .unwrap()
        .unwrap();
    assert_ne!(later.fence_id, captured.fence_id);
    assert!(policy::usage_failure_unresolved(&db, "thread-b").unwrap());
    assert!(
        !policy::finish_episode_claim(&db, "thread-b", restore, "restoring", "ordinary").unwrap()
    );
}

#[test]
fn inherited_restore_cannot_bypass_manual_off_or_archive_policy() {
    for boundary in ["manual", "off", "archive"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("fixture.sqlite");
        confirmed(&db);
        if boundary == "archive" {
            cdr_store::archive_fence::reserve(
                &db,
                &std::collections::BTreeSet::from(["thread-b".to_owned()]),
                None,
            )
            .unwrap();
        } else {
            policy::set_mode(&db, "thread-b", boundary).unwrap();
        }
        let before = policy::get(&db, "thread-b").unwrap().unwrap();
        assert!(
            policy::begin_restore_claim(&db, "thread-b", before.revision, "account-a", Some(20), 2)
                .unwrap()
                .is_none()
        );
        assert_eq!(policy::get(&db, "thread-b").unwrap().unwrap(), before);
    }
}
