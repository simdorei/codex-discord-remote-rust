use super::*;

#[test]
fn concurrent_lookup_misses_revalidate_canonical_prompt_before_attempt_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let context = ActionContext {
        channel_id: 99,
        user_id: 20,
        discord_message_id: Some(30),
        auto_queue_when_busy: true,
    };
    // Deterministic interleaving: both preliminary reads miss, A commits its
    // admission, B resumes the exact production miss branch before A starts.
    assert!(by_origin(&db, 30).unwrap().is_none());
    assert!(by_origin(&db, 30).unwrap().is_none());
    let original = admit_after_lookup_miss(&db, context, "original").unwrap();
    let conflict = admit_after_lookup_miss(&db, context, "conflicting");
    assert!(
        conflict.is_err(),
        "canonical admission must not authorize a different prompt"
    );
    let retained = by_origin(&db, 30).unwrap().unwrap();
    assert_eq!(
        retained, original,
        "rejected duplicate must not take or hold A's attempt"
    );
    assert!(cdr_store::ingress::begin_thread_start(&db, &original.ingress_id, 1, 2.0).unwrap());
    assert!(!cdr_store::ingress::begin_thread_start(&db, &original.ingress_id, 1, 3.0).unwrap());
}
