use super::*;
use cdr_store::reserve_policy::{self as reserve, admission};

#[test]
fn preparation_stamp_changed_before_claim_cannot_create_a_reply_job() {
    let (_dir, db, id) = fixture();
    queue::complete(&db, "origin").unwrap();
    reserve::ensure(&db, "thread").unwrap();
    let stamp = admission::capture(&db, "thread").unwrap();
    reserve::set_mode(&db, "thread", "on").unwrap();
    assert!(
        aq::begin_dispatch_prepared(&db, &claim(&id, aq::DispatchMode::Start), &stamp).is_err()
    );
    assert_eq!(aq::get(&db, &id).unwrap().state, "open");
    assert!(queue::list(&db).unwrap().is_empty());
}

#[test]
fn actual_dispatch_guard_accepts_only_same_reservation_and_policy() {
    let (_dir, db, id) = fixture();
    queue::complete(&db, "origin").unwrap();
    reserve::ensure(&db, "thread").unwrap();
    let stamp = admission::capture(&db, "thread").unwrap();
    aq::begin_dispatch_prepared(&db, &claim(&id, aq::DispatchMode::Start), &stamp).unwrap();
    aq::validate_dispatch_guards(&db, "thread").unwrap();
    reserve::stage_usage_failure(&db, "thread", "late fence").unwrap();
    assert!(aq::validate_dispatch_guards(&db, "thread").is_err());
    assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
    assert_eq!(
        queue::list(&db).unwrap()[0].state,
        queue::QueueJobState::Quarantined
    );
}

#[test]
fn exact_job_mutation_cannot_be_confirmed_or_deleted_by_an_old_question() {
    let (_dir, db, id) = fixture();
    queue::complete(&db, "origin").unwrap();
    aq::begin_dispatch(&db, &claim(&id, aq::DispatchMode::Start)).unwrap();
    open_initialized(&db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET prompt='another owner input'",
            [],
        )
        .unwrap();
    let changed = queue::list(&db).unwrap();
    assert!(aq::validate_dispatch_guards(&db, "thread").is_err());
    assert!(aq::confirm_dispatch(&db, &id, "accepted").is_err());
    assert!(aq::reject_definite(&db, &id, "late rejection").is_err());
    assert_eq!(queue::list(&db).unwrap(), changed);
    assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
}

#[test]
fn typed_usage_fence_failure_rolls_back_question_rejection_and_quarantine_delete() {
    let (_dir, db, id) = fixture();
    queue::complete(&db, "origin").unwrap();
    reserve::ensure(&db, "thread").unwrap();
    aq::begin_dispatch(&db, &claim(&id, aq::DispatchMode::Start)).unwrap();
    let before = queue::list(&db).unwrap();
    open_initialized(&db).unwrap().execute_batch("CREATE TRIGGER fail_usage BEFORE UPDATE ON codex_reserve_policy BEGIN SELECT RAISE(ABORT,'usage persistence failed'); END;").unwrap();
    assert!(aq::reject_usage_limit(&db, &id, "typed rejection").is_err());
    assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
    assert_eq!(queue::list(&db).unwrap(), before);
    open_initialized(&db)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_usage")
        .unwrap();
    aq::reject_usage_limit(&db, &id, "typed rejection").unwrap();
    assert!(reserve::usage_failure_unresolved(&db, "thread").unwrap());
    assert_eq!(aq::get(&db, &id).unwrap().state, "rejected");
    assert!(queue::list(&db).unwrap().is_empty());
    assert!(aq::begin_dispatch(&db, &claim(&id, aq::DispatchMode::Start)).is_err());
}

#[test]
fn manual_and_off_typed_rejection_do_not_create_an_automatic_fence() {
    for mode in ["manual", "off"] {
        let (_dir, db, id) = fixture();
        queue::complete(&db, "origin").unwrap();
        reserve::set_mode(&db, "thread", mode).unwrap();
        aq::begin_dispatch(&db, &claim(&id, aq::DispatchMode::Start)).unwrap();
        aq::reject_usage_limit(&db, &id, "typed rejection").unwrap();
        assert!(!reserve::usage_failure_unresolved(&db, "thread").unwrap());
        assert_eq!(aq::get(&db, &id).unwrap().state, "rejected");
    }
}

#[test]
fn legacy_pending_dispatch_is_preserved_without_manufacturing_a_preparation() {
    let (_dir, db, id) = fixture();
    queue::complete(&db, "origin").unwrap();
    aq::begin_dispatch(&db, &claim(&id, aq::DispatchMode::Start)).unwrap();
    open_initialized(&db)
        .unwrap()
        .execute_batch("ALTER TABLE cdr_async_questions DROP COLUMN preparation_json;")
        .unwrap();
    assert!(aq::validate_dispatch_guards(&db, "thread").is_err());
    assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
    assert_eq!(
        queue::list(&db).unwrap()[0].state,
        queue::QueueJobState::Quarantined
    );
    assert!(aq::confirm_dispatch(&db, &id, "guessed-acceptance").is_err());
}

#[test]
fn failed_acceptance_commit_preserves_one_unreplayable_reply() {
    let (_dir, db, id) = fixture();
    queue::complete(&db, "origin").unwrap();
    aq::begin_dispatch(&db, &claim(&id, aq::DispatchMode::Start)).unwrap();
    let before = queue::list(&db).unwrap();
    open_initialized(&db).unwrap().execute_batch("CREATE TRIGGER fail_confirm BEFORE UPDATE ON cdr_async_questions WHEN NEW.state='submitted' BEGIN SELECT RAISE(ABORT,'commit failure'); END;").unwrap();
    assert!(aq::confirm_dispatch(&db, &id, "accepted").is_err());
    assert_eq!(queue::list(&db).unwrap(), before);
    assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
    assert!(aq::begin_dispatch(&db, &claim(&id, aq::DispatchMode::Start)).is_err());
}
