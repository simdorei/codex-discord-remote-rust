use tempfile::tempdir;

#[test]
fn policy_episode_and_manual_off_transitions_are_durable() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("mirror.sqlite");
    let initial = cdr_store::reserve_policy::ensure(&database, "thread-1").unwrap();
    assert_eq!(initial.mode, "auto");
    assert_eq!(initial.state, "ordinary");

    assert!(
        cdr_store::reserve_policy::begin_episode(
            &database,
            "thread-1",
            "account-a",
            Some(10),
            3,
            ("gpt-5.6-luna", Some("high"), Some("default")),
        )
        .unwrap()
    );
    assert!(
        !cdr_store::reserve_policy::begin_episode(
            &database,
            "thread-1",
            "account-a",
            Some(10),
            3,
            ("other", Some("low"), None),
        )
        .unwrap()
    );
    assert!(cdr_store::reserve_policy::finish_episode(&database, "thread-1", "reserve").unwrap());
    let transition_notice = cdr_store::reserve_policy::transition_notice::pending(&database)
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        transition_notice
            .content
            .contains("not recorded (legacy episode API)")
    );

    let disabled = cdr_store::reserve_policy::set_mode(&database, "thread-1", "off").unwrap();
    assert_eq!(disabled.state, "reserve");
    assert_eq!(disabled.previous_model.as_deref(), Some("gpt-5.6-luna"));

    let manual = cdr_store::reserve_policy::set_mode(&database, "thread-1", "manual").unwrap();
    assert_eq!(manual.state, "ordinary");
    assert_eq!(manual.previous_model, None);
}

#[test]
fn usage_failure_fence_is_durable_and_resolved_by_exact_policy_progress() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("mirror.sqlite");
    cdr_store::reserve_policy::ensure(&database, "thread-1").unwrap();

    cdr_store::reserve_policy::stage_usage_failure(&database, "thread-1", "typed usage failure")
        .unwrap();
    assert!(cdr_store::reserve_policy::usage_failure_unresolved(&database, "thread-1").unwrap());
    let claim = cdr_store::reserve_policy::usage_failure_claim(&database, "thread-1")
        .unwrap()
        .unwrap();
    assert!(
        cdr_store::reserve_policy::resolve_usage_failure(
            &database,
            "thread-1",
            claim.fence_id,
            claim.policy_revision,
            "confirmed ordinary quota",
        )
        .unwrap()
    );
    assert!(!cdr_store::reserve_policy::usage_failure_unresolved(&database, "thread-1").unwrap());
}

#[test]
fn usage_failure_fence_rejects_stale_claims_and_preserves_off_until_manual_override() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("mirror.sqlite");
    cdr_store::reserve_policy::ensure(&database, "thread-1").unwrap();

    cdr_store::reserve_policy::stage_usage_failure(&database, "thread-1", "failure one").unwrap();
    let first = cdr_store::reserve_policy::usage_failure_claim(&database, "thread-1")
        .unwrap()
        .unwrap();
    cdr_store::reserve_policy::stage_usage_failure(&database, "thread-1", "failure two").unwrap();
    let second = cdr_store::reserve_policy::usage_failure_claim(&database, "thread-1")
        .unwrap()
        .unwrap();
    assert_ne!(first.fence_id, second.fence_id);
    assert!(
        !cdr_store::reserve_policy::resolve_usage_failure(
            &database,
            "thread-1",
            first.fence_id,
            first.policy_revision,
            "stale resolution",
        )
        .unwrap()
    );
    assert!(cdr_store::reserve_policy::usage_failure_unresolved(&database, "thread-1").unwrap());

    let off = cdr_store::reserve_policy::set_mode(&database, "thread-1", "off").unwrap();
    assert_eq!(off.mode, "off");
    assert!(cdr_store::reserve_policy::usage_failure_unresolved(&database, "thread-1").unwrap());
    assert!(
        !cdr_store::reserve_policy::resolve_usage_failure(
            &database,
            "thread-1",
            second.fence_id,
            second.policy_revision,
            "stale off resolution",
        )
        .unwrap()
    );

    cdr_store::reserve_policy::set_mode(&database, "thread-1", "manual").unwrap();
    assert!(!cdr_store::reserve_policy::usage_failure_unresolved(&database, "thread-1").unwrap());
}

#[test]
fn episode_completion_resolves_the_failure_claim_captured_before_revision_advance() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("mirror.sqlite");
    let initial = cdr_store::reserve_policy::ensure(&database, "thread-1").unwrap();
    cdr_store::reserve_policy::stage_usage_failure(&database, "thread-1", "failure one").unwrap();

    let episode = cdr_store::reserve_policy::begin_episode_claim(
        &database,
        "thread-1",
        initial.revision,
        cdr_store::reserve_policy::EpisodeIdentity {
            account_id: "account-a",
            process_id: Some(10),
            generation: 3,
        },
        ("model-a", Some("high"), Some("default")),
        true,
        (Some("gpt-reserve"), Some("high"), Some("default")),
    )
    .unwrap()
    .unwrap();
    assert!(
        episode
            .usage_failure
            .unwrap()
            .failure_policy_revision
            .is_some()
    );
    assert!(
        cdr_store::reserve_policy::finish_episode_claim(
            &database, "thread-1", episode, "entering", "reserve",
        )
        .unwrap()
    );
    assert!(!cdr_store::reserve_policy::usage_failure_unresolved(&database, "thread-1").unwrap());
}

#[test]
fn episode_completion_does_not_resolve_a_failure_staged_after_episode_begin() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("mirror.sqlite");
    let initial = cdr_store::reserve_policy::ensure(&database, "thread-1").unwrap();
    cdr_store::reserve_policy::stage_usage_failure(&database, "thread-1", "failure one").unwrap();
    let episode = cdr_store::reserve_policy::begin_episode_claim(
        &database,
        "thread-1",
        initial.revision,
        cdr_store::reserve_policy::EpisodeIdentity {
            account_id: "account-a",
            process_id: Some(10),
            generation: 3,
        },
        ("model-a", Some("high"), Some("default")),
        true,
        (Some("gpt-reserve"), Some("high"), Some("default")),
    )
    .unwrap()
    .unwrap();
    let first = episode.usage_failure.unwrap();
    cdr_store::reserve_policy::stage_usage_failure(&database, "thread-1", "failure two").unwrap();

    assert!(
        cdr_store::reserve_policy::finish_episode_claim(
            &database, "thread-1", episode, "entering", "reserve",
        )
        .unwrap()
    );
    let current = cdr_store::reserve_policy::usage_failure_claim(&database, "thread-1")
        .unwrap()
        .unwrap();
    assert_ne!(current.fence_id, first.fence_id);
    assert!(cdr_store::reserve_policy::usage_failure_unresolved(&database, "thread-1").unwrap());
}
