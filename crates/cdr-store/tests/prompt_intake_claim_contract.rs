use std::sync::{Arc, Barrier};

use cdr_store::StoreError;
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, get_prompt_intake, list_ready_prompt_intakes,
    record_prompt_intake_failure_if_claimed, renew_prompt_intake_claim_if_current,
    try_claim_prompt_intake,
};

#[test]
fn claim_lease_excludes_concurrent_recovery_and_stale_owner_after_crash_expiry() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("claim.sqlite");
    admit_prompt_intake(&path, intake("job", 60)).unwrap();

    let first = try_claim_prompt_intake(&path, "job", 10.0, 20.0)
        .unwrap()
        .unwrap();
    assert!(
        try_claim_prompt_intake(&path, "job", 10.0, 30.0)
            .unwrap()
            .is_none()
    );
    assert!(list_ready_prompt_intakes(&path, 19.999).unwrap().is_empty());
    assert_eq!(list_ready_prompt_intakes(&path, 20.0).unwrap().len(), 1);

    let recovered = try_claim_prompt_intake(&path, "job", 20.0, 40.0)
        .unwrap()
        .unwrap();
    assert_ne!(first.claim_token, recovered.claim_token);
    assert!(
        record_prompt_intake_failure_if_claimed(&path, &first, "late failure", 50.0)
            .unwrap()
            .is_none()
    );
    let current = get_prompt_intake(&path, "job").unwrap().unwrap();
    assert_eq!(
        current.claim_token.as_deref(),
        Some(&*recovered.claim_token)
    );
    assert!(current.last_error.is_empty());
    assert_eq!(current.attempt_count, 0);
}

#[test]
fn simultaneous_recovery_claims_have_exactly_one_winner() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("concurrent-claim.sqlite");
    admit_prompt_intake(&path, intake("job", 61)).unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let workers: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                try_claim_prompt_intake(&path, "job", 10.0, 610.0).unwrap()
            })
        })
        .collect();
    barrier.wait();
    let claims: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();

    assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
    assert!(
        list_ready_prompt_intakes(&path, 609.999)
            .unwrap()
            .is_empty()
    );
    assert_eq!(list_ready_prompt_intakes(&path, 610.0).unwrap().len(), 1);
}

#[test]
fn failure_metadata_is_bounded_and_duplicate_admission_does_not_clear_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bounded-failure.sqlite");
    admit_prompt_intake(&path, intake("job", 62)).unwrap();
    let claim = try_claim_prompt_intake(&path, "job", 1.0, 601.0)
        .unwrap()
        .unwrap();
    let failed = record_prompt_intake_failure_if_claimed(&path, &claim, &"x".repeat(1_500), 700.0)
        .unwrap()
        .unwrap();
    assert_eq!(failed.last_error.chars().count(), 1_000);
    assert_eq!(failed.attempt_count, 1);
    assert!((failed.retry_after - 700.0).abs() < f64::EPSILON);

    let duplicate = admit_prompt_intake(&path, intake("job", 62)).unwrap();
    assert!(!duplicate.created);
    assert_eq!(duplicate.intake, failed);
}

#[test]
fn a_live_owner_can_extend_its_lease() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("renew-live.sqlite");
    admit_prompt_intake(&path, intake("job", 63)).unwrap();
    let claim = try_claim_prompt_intake(&path, "job", 10.0, 20.0)
        .unwrap()
        .unwrap();

    assert!(matches!(
        renew_prompt_intake_claim_if_current(&path, &claim, 15.0, 15.0),
        Err(StoreError::InvalidPromptIntakeLease { .. })
    ));

    let renewed = renew_prompt_intake_claim_if_current(&path, &claim, 15.0, 30.0)
        .unwrap()
        .expect("live token renews");
    assert_eq!(renewed.claim_token, claim.claim_token);
    assert!((renewed.intake.claim_expires_at - 30.0).abs() < f64::EPSILON);
    assert!(list_ready_prompt_intakes(&path, 29.999).unwrap().is_empty());
    assert_eq!(list_ready_prompt_intakes(&path, 30.0).unwrap().len(), 1);
}

#[test]
fn a_stale_token_cannot_renew_a_live_lease() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("renew-stale-token.sqlite");
    admit_prompt_intake(&path, intake("job", 64)).unwrap();
    let claim = try_claim_prompt_intake(&path, "job", 10.0, 20.0)
        .unwrap()
        .unwrap();
    let mut stale = claim.clone();
    stale.claim_token = "not-the-owner".to_owned();

    assert!(
        renew_prompt_intake_claim_if_current(&path, &stale, 15.0, 30.0)
            .unwrap()
            .is_none()
    );
    let current = get_prompt_intake(&path, "job").unwrap().unwrap();
    assert_eq!(current.claim_token.as_deref(), Some(&*claim.claim_token));
    assert!((current.claim_expires_at - 20.0).abs() < f64::EPSILON);
}

#[test]
fn expired_then_reclaimed_intake_rejects_the_old_owners_renewal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("renew-reclaimed.sqlite");
    admit_prompt_intake(&path, intake("job", 65)).unwrap();
    let expired = try_claim_prompt_intake(&path, "job", 10.0, 20.0)
        .unwrap()
        .unwrap();
    assert!(
        renew_prompt_intake_claim_if_current(&path, &expired, 20.0, 50.0)
            .unwrap()
            .is_none()
    );
    let current = try_claim_prompt_intake(&path, "job", 20.0, 40.0)
        .unwrap()
        .unwrap();

    assert!(
        renew_prompt_intake_claim_if_current(&path, &expired, 21.0, 50.0)
            .unwrap()
            .is_none()
    );
    let stored = get_prompt_intake(&path, "job").unwrap().unwrap();
    assert_eq!(stored.claim_token.as_deref(), Some(&*current.claim_token));
    assert!((stored.claim_expires_at - 40.0).abs() < f64::EPSILON);
}

fn intake(job_id: &str, message_id: i64) -> NewPromptIntake<'_> {
    NewPromptIntake {
        job_id,
        target_thread_id: "source",
        channel_id: 101,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        raw_prompt: "attachment-enriched raw prompt",
        auto_queue_when_busy: true,
        require_current_mirror: true,
        created_at: 1.0,
    }
}
