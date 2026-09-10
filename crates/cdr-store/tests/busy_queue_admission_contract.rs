use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use cdr_store::StoreError;
use cdr_store::claims::{
    BusyChoice, NewBusyChoice, claim_busy_choice, cleanup_busy_choices, component_claim_counts,
    create_busy_choice, get_busy_choice,
};
use cdr_store::prompt_intake::{admit_busy_queue, list_prompt_intakes};
use cdr_store::schema::open_initialized;

#[test]
fn queue_acceptance_saves_raw_prompt_and_receipt_together() {
    let (_temp, path, choice) = fixture();
    let accepted =
        admit_busy_queue(&path, &choice, "displayed-target", false, "receipt", 101.0).unwrap();
    let intake = accepted.intake.unwrap();
    assert_eq!(accepted.job_id, format!("busy-choice:{}", choice.choice_id));
    assert_eq!(intake.job_id, accepted.job_id);
    assert_eq!(intake.target_thread_id, "displayed-target");
    assert_eq!(intake.raw_prompt, "보존할 원문\nattachment context");
    assert_eq!(intake.channel_id, 10);
    assert_eq!(intake.owner_user_id, Some(20));
    assert!(intake.auto_queue_when_busy);
    assert!(!intake.require_current_mirror);
    assert_eq!(list_prompt_intakes(&path).unwrap(), vec![intake]);
    assert_eq!(component_claim_counts(&path, 101.0).unwrap(), (1, 0));
    assert!(!claim_busy_choice(&path, &choice.choice_id, 101.0).unwrap());
}

#[test]
fn repeated_click_after_cleanup_and_completion_does_not_recreate_intake() {
    let (_temp, path, choice) = fixture();
    let accepted =
        admit_busy_queue(&path, &choice, "displayed-target", false, "receipt", 101.0).unwrap();
    assert_eq!(cleanup_busy_choices(&path, 102.0).unwrap(), 0);
    // An in-flight claim is retained until the original button expires.
    // The success receipt was created later and remains valid across cleanup.
    assert_eq!(cleanup_busy_choices(&path, 1_900.0).unwrap(), 1);
    open_initialized(&path)
        .unwrap()
        .execute("DELETE FROM codex_prompt_intakes", [])
        .unwrap();
    let repeated = admit_busy_queue(
        &path,
        &choice,
        "displayed-target",
        false,
        "receipt",
        1_900.5,
    )
    .unwrap();
    assert_eq!(repeated.job_id, accepted.job_id);
    assert!(repeated.intake.is_none());
    assert!(list_prompt_intakes(&path).unwrap().is_empty());
    // Once the receipt expires, the missing/expired button cannot be admitted again.
    assert!(matches!(
        admit_busy_queue(
            &path,
            &choice,
            "displayed-target",
            false,
            "receipt",
            2_000.0
        ),
        Err(StoreError::BusyChoiceUnavailable(_))
    ));
}

#[test]
fn failure_at_either_write_rolls_back_the_claim_intake_and_receipt() {
    for table in ["codex_prompt_intakes", "persistent_component_claims"] {
        let (_temp, path, choice) = fixture();
        let connection = open_initialized(&path).unwrap();
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER reject_acceptance BEFORE INSERT ON {table} \
                 BEGIN SELECT RAISE(ABORT, 'injected admission failure'); END;"
            ))
            .unwrap();
        let failed = admit_busy_queue(&path, &choice, "displayed-target", false, "receipt", 101.0);
        assert!(matches!(&failed, Err(StoreError::Database(_))));
        assert!(
            failed
                .err()
                .unwrap()
                .to_string()
                .contains("injected admission failure")
        );
        assert!(list_prompt_intakes(&path).unwrap().is_empty());
        assert_eq!(component_claim_counts(&path, 101.0).unwrap(), (0, 0));
        assert_eq!(
            get_busy_choice(&path, &choice.choice_id, 101.0).unwrap(),
            Some(choice.clone())
        );

        connection
            .execute_batch("DROP TRIGGER reject_acceptance")
            .unwrap();
        assert!(
            admit_busy_queue(&path, &choice, "displayed-target", false, "receipt", 102.0)
                .unwrap()
                .intake
                .is_some()
        );
    }
}

#[test]
fn stale_expired_or_control_claimed_choice_never_creates_a_queue_intake() {
    for change in 0..8 {
        let (_temp, path, choice) = fixture();
        let mut submitted = choice.clone();
        let mut now = 101.0;
        match change {
            0 => submitted.owner_user_id += 1,
            1 => submitted.channel_id += 1,
            2 => submitted.prompt.push_str("changed"),
            3 => submitted.target_thread_id = Some("changed".into()),
            4 => submitted.allow_steer = !submitted.allow_steer,
            5 => submitted.expires_at += 1.0,
            6 => now = choice.expires_at,
            7 => assert!(claim_busy_choice(&path, &choice.choice_id, now).unwrap()),
            _ => unreachable!(),
        }
        assert!(matches!(
            admit_busy_queue(&path, &submitted, "displayed-target", false, "receipt", now),
            Err(StoreError::BusyChoiceUnavailable(_))
        ));
        assert!(list_prompt_intakes(&path).unwrap().is_empty());
        assert_eq!(component_claim_counts(&path, now).unwrap(), (0, 0));
    }
}

#[test]
fn concurrent_clicks_have_exactly_one_preparation_owner() {
    let (_temp, path, choice) = fixture();
    let barrier = Arc::new(Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let (path, choice, barrier) = (path.clone(), choice.clone(), Arc::clone(&barrier));
            std::thread::spawn(move || {
                barrier.wait();
                admit_busy_queue(&path, &choice, "displayed-target", false, "receipt", 101.0)
                    .unwrap()
                    .intake
                    .is_some()
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|thread| usize::from(thread.join().unwrap()))
            .sum::<usize>(),
        1
    );
    assert_eq!(list_prompt_intakes(&path).unwrap().len(), 1);
    assert_eq!(component_claim_counts(&path, 101.0).unwrap(), (1, 0));
}

fn fixture() -> (tempfile::TempDir, PathBuf, BusyChoice) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mirror.sqlite");
    let choice = choice(&path);
    (temp, path, choice)
}

fn choice(path: &Path) -> BusyChoice {
    let id = create_busy_choice(
        path,
        NewBusyChoice {
            owner_user_id: 20,
            channel_id: 10,
            target_thread_id: Some("displayed-target"),
            prompt: "보존할 원문\nattachment context",
            allow_steer: true,
            now: 100.0,
            time_to_live: 1_800.0,
        },
    )
    .unwrap();
    get_busy_choice(path, &id, 100.0).unwrap().unwrap()
}
