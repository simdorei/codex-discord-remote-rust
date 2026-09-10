#[path = "support/new_reply_fixture.rs"]
mod support;
use cdr_store::{
    delivery,
    delivery_receipt::{self, ReceiptState},
    ingress,
    new_reply::{self, DeliveryGuard},
    queue,
};
use serde_json::json;
use support::{Fixture, hash};

#[test]
fn warning_definite_rejection_can_retry_but_unknown_cannot() {
    let f = Fixture::new();
    f.accept();
    let initial = new_reply::get(&f.db, "job").unwrap().unwrap();
    new_reply::checkpoint(
        &f.db,
        &initial,
        new_reply::CheckpointUpdate {
            scan: &serde_json::json!({}),
            verified: false,
            error: "",
            now: initial.accepted_at.unwrap() + 121.0,
        },
    )
    .unwrap();
    let record = new_reply::get(&f.db, "job").unwrap().unwrap();
    let key = json!([99, "new/verification-notice/v1", "job", 0]).to_string();
    let content_hash = hash(&new_reply::warning_text(&record));
    assert_eq!(
        delivery_receipt::begin(&f.db, &key, &content_hash).unwrap(),
        ReceiptState::New
    );
    assert_eq!(
        new_reply::get(&f.db, "job").unwrap().unwrap().warning_due,
        2
    );
    assert_eq!(
        delivery_receipt::begin(&f.db, &key, &content_hash).unwrap(),
        ReceiptState::Unknown
    );
    assert_eq!(
        new_reply::get(&f.db, "job").unwrap().unwrap().warning_due,
        2
    );
    assert!(delivery_receipt::release_rejected(&f.db, &key).unwrap());
    assert_eq!(
        new_reply::get(&f.db, "job").unwrap().unwrap().warning_due,
        1
    );
    assert_eq!(
        delivery_receipt::begin(&f.db, &key, &content_hash).unwrap(),
        ReceiptState::New
    );
}

#[test]
fn intent_precedes_start_and_survives_completion_without_action_result() {
    let f = Fixture::new();
    let before = new_reply::get(&f.db, "job").unwrap().unwrap();
    assert!(before.turn_id.is_none());
    assert!(before.accepted_at.is_none());
    f.accept();
    let accepted = new_reply::get(&f.db, "job").unwrap().unwrap();
    assert_eq!(accepted.turn_id.as_deref(), Some("first-turn"));
    delivery::stage_queue_completion(&f.db, "job", "Final\nanswer", 9.0).unwrap();
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert_eq!(new_reply::pending(&f.db, 8).unwrap().len(), 1);
    assert_eq!(new_reply::get(&f.db, "job").unwrap().unwrap(), accepted);
    assert_ne!(
        ingress::get(&f.db, "message:30").unwrap().unwrap().phase,
        "result_recorded"
    );
}

#[test]
fn first_turn_binding_and_running_state_commit_or_roll_back_together() {
    let f = Fixture::new();
    rusqlite::Connection::open(&f.db).unwrap().execute_batch(
        "CREATE TRIGGER reject_bind BEFORE UPDATE OF turn_id ON codex_new_first_replies BEGIN SELECT RAISE(ABORT,'injected binding failure'); END;"
    ).unwrap();
    let starting = queue::try_begin_attempt(&f.db, "job", &[], 1)
        .unwrap()
        .unwrap();
    assert!(
        queue::mark_running_if_claimed(&f.db, &starting, "first-turn")
            .unwrap_err()
            .to_string()
            .contains("injected")
    );
    assert_eq!(
        queue::list(&f.db).unwrap()[0].state,
        queue::QueueJobState::Starting
    );
    assert!(
        new_reply::get(&f.db, "job")
            .unwrap()
            .unwrap()
            .turn_id
            .is_none()
    );
}

#[test]
fn all_six_ack_verification_outbox_orders_require_both_proofs() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let f = Fixture::new();
        f.accept();
        let (mut ack, mut verified, mut staged) = (false, false, false);
        for step in order {
            match step {
                0 => {
                    f.ack();
                    ack = true;
                }
                1 => {
                    f.verify();
                    verified = true;
                }
                _ => {
                    delivery::stage_queue_completion(&f.db, "job", "Final\nanswer", 9.0).unwrap();
                    staged = true;
                }
            }
            if staged {
                let key = json!([100, "completion/v1", "job", 0]).to_string();
                let state = delivery_receipt::begin_guarded(
                    &f.db,
                    &key,
                    &hash("Final\nanswer"),
                    Some(&DeliveryGuard {
                        job_id: "job",
                        thread_id: "new-thread",
                        turn_id: "first-turn",
                    }),
                )
                .unwrap();
                if ack && verified {
                    assert_eq!(state, ReceiptState::New);
                } else {
                    assert!(matches!(state, ReceiptState::Held(_)));
                }
            }
        }
    }
}

#[test]
fn error_receipt_and_bare_confirmation_are_not_normal_acknowledgement() {
    let f = Fixture::new();
    f.accept();
    f.verify();
    ingress::record_result(&f.db, "message:30", &json!({"response":"error"}), 8.0).unwrap();
    let key = json!([99, "message/error/v1", "inbound-message/30/error-report", 0]).to_string();
    delivery_receipt::begin(&f.db, &key, &hash("error")).unwrap();
    delivery_receipt::confirm(&f.db, &key, "error-message").unwrap();
    assert!(ingress::confirm(&f.db, "message:30", 9.0).is_err());
    assert!(new_reply::output_hold(&f.db, "job").unwrap().is_some());
}

#[test]
fn concurrent_ack_senders_share_one_claim_and_unknown_never_resets() {
    let f = Fixture::new();
    f.accept();
    let record = new_reply::get(&f.db, "job").unwrap().unwrap();
    let key = new_reply::acknowledgement_key(&record).unwrap();
    let digest = hash(&record.identity.acknowledgement);
    std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|_| scope.spawn(|| delivery_receipt::begin(&f.db, &key, &digest).unwrap()))
            .collect::<Vec<_>>();
        let states = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            states.iter().filter(|s| **s == ReceiptState::New).count(),
            1
        );
        assert_eq!(
            states
                .iter()
                .filter(|s| **s == ReceiptState::Unknown)
                .count(),
            7
        );
    });
    assert_eq!(
        delivery_receipt::begin(&f.db, &key, &digest).unwrap(),
        ReceiptState::Unknown
    );
    assert!(
        !new_reply::get(&f.db, "job")
            .unwrap()
            .unwrap()
            .confirmation_delivered
    );
}

#[test]
fn mapping_changed_after_preview_blocks_actual_send_claim() {
    let f = Fixture::new();
    f.accept();
    f.verify();
    f.ack();
    assert!(new_reply::output_hold(&f.db, "job").unwrap().is_none());
    cdr_store::mapping::update_discord_thread_id(&f.db, "new-thread", 101, 10.0).unwrap();
    let key = json!([100, "completion/v1", "job", 0]).to_string();
    assert!(
        delivery_receipt::begin_guarded(
            &f.db,
            &key,
            &hash("Final"),
            Some(&DeliveryGuard {
                job_id: "job",
                thread_id: "new-thread",
                turn_id: "first-turn",
            })
        )
        .is_err()
    );
}

#[test]
fn warning_deadline_survives_restart_and_stale_scan_cannot_commit() {
    let f = Fixture::new();
    f.accept();
    let before = new_reply::get(&f.db, "job").unwrap().unwrap();
    let accepted = before.accepted_at.unwrap();
    assert!(
        new_reply::checkpoint(
            &f.db,
            &before,
            new_reply::CheckpointUpdate {
                scan: &json!({}),
                verified: false,
                error: "",
                now: accepted + 119.0
            }
        )
        .unwrap()
    );
    assert_eq!(
        new_reply::get(&f.db, "job").unwrap().unwrap().warning_due,
        0
    );
    assert!(
        !new_reply::checkpoint(
            &f.db,
            &before,
            new_reply::CheckpointUpdate {
                scan: &json!({}),
                verified: true,
                error: "",
                now: accepted + 119.5
            }
        )
        .unwrap()
    );
    let after = new_reply::get(&f.db, "job").unwrap().unwrap();
    assert!(
        new_reply::checkpoint(
            &f.db,
            &after,
            new_reply::CheckpointUpdate {
                scan: &json!({}),
                verified: false,
                error: "disk read failed",
                now: accepted + 120.0
            }
        )
        .unwrap()
    );
    let late = new_reply::get(&f.db, "job").unwrap().unwrap();
    assert_eq!(late.warning_due, 1);
    assert_eq!(late.last_error, "disk read failed");
    assert_eq!(late.accepted_at, Some(accepted));
    assert_eq!(
        queue::list(&f.db).unwrap()[0].state,
        queue::QueueJobState::Running
    );
}
