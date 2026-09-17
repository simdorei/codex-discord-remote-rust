use cdr_store::reserve_policy::{self, EpisodeIdentity, Policy, alignment};
use std::path::Path;

fn identity() -> EpisodeIdentity<'static> {
    EpisodeIdentity {
        account_id: "account-a",
        process_id: Some(9),
        generation: 2,
    }
}

fn reserve(db: &Path) -> Policy {
    let p = reserve_policy::ensure(db, "thread").unwrap();
    let claim = reserve_policy::begin_episode_claim(
        db,
        "thread",
        p.revision,
        identity(),
        ("original", Some("high"), Some("priority")),
        true,
        (Some("gpt-reserve"), Some("medium"), Some("default")),
    )
    .unwrap()
    .unwrap();
    assert!(
        reserve_policy::finish_episode_claim(db, "thread", claim, "entering", "reserve").unwrap()
    );
    reserve_policy::get(db, "thread").unwrap().unwrap()
}

fn current(db: &Path) -> Policy {
    reserve_policy::get(db, "thread").unwrap().unwrap()
}

#[test]
fn r15_alignment_captures_fence_and_preserves_original_restore_settings() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("fixture.sqlite");
    let before = reserve(&db);
    reserve_policy::stage_usage_failure(&db, "thread", "prior failure").unwrap();
    let claim = alignment::begin(&db, &before, identity(), "xhigh")
        .unwrap()
        .unwrap();
    let pending = current(&db);
    assert_eq!(pending.state, "entering");
    assert_eq!(pending.previous_model, before.previous_model);
    assert_eq!(pending.previous_effort, before.previous_effort);
    assert_eq!(pending.previous_tier, before.previous_tier);
    assert_eq!(
        pending.previous_effort_present,
        before.previous_effort_present
    );
    assert_eq!(pending.applied_effort.as_deref(), Some("xhigh"));
    assert!(reserve_policy::usage_failure_unresolved(&db, "thread").unwrap());
    assert!(alignment::finish(&db, &claim).unwrap());
    assert_eq!(current(&db).state, "reserve");
    assert!(!reserve_policy::usage_failure_unresolved(&db, "thread").unwrap());
    let finished = current(&db);
    assert!(!alignment::finish(&db, &claim).unwrap());
    assert_eq!(current(&db), finished);
}

#[test]
fn r15_alignment_without_episode_does_not_invent_an_ordinary_restore() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("fixture.sqlite");
    let before = reserve_policy::ensure(&db, "thread").unwrap();
    let claim = alignment::begin(&db, &before, identity(), "medium")
        .unwrap()
        .unwrap();
    assert!(alignment::finish(&db, &claim).unwrap());
    let after = current(&db);
    assert_eq!(after.state, "ordinary");
    assert!(after.previous_model.is_none());
    assert!(after.previous_effort.is_none());
    assert!(!after.previous_effort_present);
    assert!(after.previous_tier.is_none());
}

#[test]
fn r15_alignment_begin_rejects_stale_policy_disabled_mode_and_below_floor() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("fixture.sqlite");
    let before = reserve(&db);
    for effort in ["low", "minimal", "none", "med", "unknown"] {
        assert!(
            alignment::begin(&db, &before, identity(), effort)
                .unwrap()
                .is_none()
        );
        assert_eq!(current(&db), before);
    }
    let mut altered = before.clone();
    altered.previous_model = Some("foreign".into());
    assert!(
        alignment::begin(&db, &altered, identity(), "high")
            .unwrap()
            .is_none()
    );
    let off = reserve_policy::set_mode(&db, "thread", "off").unwrap();
    assert!(
        alignment::begin(&db, &before, identity(), "high")
            .unwrap()
            .is_none()
    );
    assert!(
        alignment::begin(&db, &off, identity(), "high")
            .unwrap()
            .is_none()
    );
    assert_eq!(current(&db), off);
}

#[test]
fn r15_alignment_finish_cannot_resolve_a_newer_fence_even_without_an_initial_fence() {
    for initial_fence in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("fixture.sqlite");
        let before = reserve(&db);
        if initial_fence {
            reserve_policy::stage_usage_failure(&db, "thread", "initial").unwrap();
        }
        let claim = alignment::begin(&db, &before, identity(), "high")
            .unwrap()
            .unwrap();
        reserve_policy::stage_usage_failure(&db, "thread", "newer failure").unwrap();
        let pending = current(&db);
        let fence = reserve_policy::usage_failure_claim(&db, "thread").unwrap();
        assert!(!alignment::finish(&db, &claim).unwrap());
        assert_eq!(current(&db), pending, "success state must roll back too");
        assert_eq!(
            reserve_policy::usage_failure_claim(&db, "thread").unwrap(),
            fence
        );
    }
}

#[test]
fn r15_alignment_legacy_null_fence_stays_pending_and_rolls_back_success() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("fixture.sqlite");
    let before = reserve(&db);
    reserve_policy::stage_usage_failure(&db, "thread", "legacy failure").unwrap();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE codex_reserve_policy SET usage_failure_revision=NULL WHERE thread_id='thread'",
            [],
        )
        .unwrap();
    let claim = alignment::begin(&db, &before, identity(), "high")
        .unwrap()
        .unwrap();
    let pending = current(&db);
    assert!(!alignment::finish(&db, &claim).unwrap());
    assert_eq!(current(&db), pending);
    assert!(reserve_policy::usage_failure_unresolved(&db, "thread").unwrap());
}

#[test]
fn r15_alignment_sql_abort_preserves_intent_and_usage_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("fixture.sqlite");
    let before = reserve(&db);
    reserve_policy::stage_usage_failure(&db, "thread", "prior failure").unwrap();
    let claim = alignment::begin(&db, &before, identity(), "high")
        .unwrap()
        .unwrap();
    let pending = current(&db);
    let fence = reserve_policy::usage_failure_claim(&db, "thread").unwrap();
    rusqlite::Connection::open(&db).unwrap().execute_batch(
        "CREATE TRIGGER fail_resolve BEFORE UPDATE OF usage_failure_state ON codex_reserve_policy
        WHEN NEW.usage_failure_state='resolved' BEGIN SELECT RAISE(ABORT,'fixture finish failure'); END;",
    ).unwrap();
    assert!(alignment::finish(&db, &claim).is_err());
    assert_eq!(current(&db), pending);
    assert_eq!(
        reserve_policy::usage_failure_claim(&db, "thread").unwrap(),
        fence
    );
}

#[test]
fn r15_stale_alignment_completion_does_not_overwrite_off_or_manual_override() {
    for mode in ["off", "manual"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("fixture.sqlite");
        let before = reserve(&db);
        let claim = alignment::begin(&db, &before, identity(), "high")
            .unwrap()
            .unwrap();
        let overridden = reserve_policy::set_mode(&db, "thread", mode).unwrap();
        assert!(!alignment::finish(&db, &claim).unwrap());
        assert_eq!(current(&db), overridden);
        assert!(
            !reserve_policy::mark_unknown_claim(&db, "thread", claim.revision(), "stale").unwrap()
        );
        assert_eq!(current(&db), overridden);
    }
}
