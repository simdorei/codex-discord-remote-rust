use super::*;
use crate::{delivery_receipt as receipt, new_reply::DeliveryGuard, queue};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Request) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    crate::mapping::upsert_thread(&path, "thread", "project", "title", 100, 42, 1.0).unwrap();
    queue::enqueue(
        &path,
        queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(11),
            app_server_generation: 1,
            prompt: "original request",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(&path, "job", &[], 1).unwrap();
    queue::mark_running(&path, "job", "turn", 1).unwrap();
    let pending =
        crate::delivery::stage_queue_completion(&path, "job", "Saved answer", 3.0).unwrap();
    let db = open_initialized(&path).unwrap();
    db.execute_batch("INSERT INTO discord_ingress_journal
        (ingress_id,kind,event_id,channel_id,owner_user_id,payload_json,state,phase,
        target_thread_id,owner_kind,owner_id,confirmation_delivered,created_at,updated_at)
        VALUES ('message:11','message',11,42,3,'{}','owned','durable_prompt','thread','prompt','job',0,1,1)").unwrap();
    let key = json!([42, "message/error/v1", "inbound-message/11/error-report", 0]).to_string();
    receipt::begin(&path, &key, "error-hash").unwrap();
    receipt::confirm(&path, &key, "123").unwrap();
    let request = Request {
        delivery_id: pending.delivery_id,
        job_id: "job".into(),
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        channel_id: 42,
        original_sha256: sha256("Saved answer"),
        ingress_id: "message:11".into(),
        error_receipt_key: key,
        error_message_id: "123".into(),
        error_sha256: "error-hash".into(),
    };
    (dir, path, request)
}
fn render(s: &str) -> Vec<String> {
    vec![s.to_owned()]
}
fn claim(path: &Path, r: &Request, index: usize, hash: &str) -> Result<receipt::ReceiptState> {
    receipt::begin_guarded(
        path,
        &json!([r.channel_id, "completion/v1", r.delivery_id, index]).to_string(),
        hash,
        Some(&DeliveryGuard {
            job_id: &r.job_id,
            thread_id: &r.thread_id,
            turn_id: &r.turn_id,
        }),
    )
}
#[test]
fn final_grant_freezes_only_the_saved_result_and_deduplicates_after_reconstruction() {
    let (_dir, path, r) = fixture();
    assert!(crate::first_reply::pending(&path, "job").unwrap().is_some());
    authorize(&path, &r, render).unwrap();
    authorize(&path, &r, render).unwrap();
    let pending = crate::delivery::list_pending(&path).unwrap().remove(0);
    assert!(pending.content.starts_with(EXPLANATION));
    assert!(authorized(&path, &pending).unwrap());
    assert_eq!(
        crate::first_reply::pending(&path, "job")
            .unwrap()
            .as_deref(),
        Some("message:11")
    );
    let hash = sha256(&pending.content);
    assert_eq!(
        claim(&path, &r, 0, &hash).unwrap(),
        receipt::ReceiptState::New
    );
    assert_eq!(
        claim(&path, &r, 0, &hash).unwrap(),
        receipt::ReceiptState::Unknown
    );
    authorize(&path, &r, render).unwrap(); // no change or second intent
    let key = json!([42, "completion/v1", r.delivery_id, 0]).to_string();
    receipt::confirm(&path, &key, "456").unwrap();
    assert_eq!(
        claim(&path, &r, 0, &hash).unwrap(),
        receipt::ReceiptState::Delivered("456".into())
    );
    assert!(claim(&path, &r, 0, "changed payload").is_err());
    assert!(queue::list(&path).unwrap().is_empty());
}
#[test]
fn existing_intent_or_wrong_error_owner_cannot_authorize_any_recovery() {
    for corruption in ["intent", "error", "source", "payload", "progress"] {
        let (_dir, path, mut r) = fixture();
        let db = open_initialized(&path).unwrap();
        match corruption {
            "intent" => {
                db.execute("INSERT INTO codex_delivery_receipts(receipt_key,content_hash) VALUES (?,'hash')",
                [json!([999,"completion/v1",r.delivery_id,99]).to_string()]).unwrap();
            }
            "error" => r.error_message_id = "another-message".into(),
            "source" => r.ingress_id = "message:12".into(),
            "payload" => r.original_sha256 = "wrong".into(),
            _ => {
                db.execute_batch("INSERT INTO codex_commentary_outbox(delivery_key,job_id,target_thread_id,turn_id,channel_id,text)
                VALUES ('progress','job','thread','turn',42,'before final')").unwrap();
            }
        }
        assert!(authorize(&path, &r, render).is_err(), "{corruption}");
        assert_eq!(
            crate::delivery::list_pending(&path).unwrap()[0].content,
            "Saved answer"
        );
    }
}
#[test]
fn grant_transaction_rolls_back_the_explanation_and_actual_claim_rechecks_evidence() {
    let (_dir, path, r) = fixture();
    let db = open_initialized(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_grant BEFORE INSERT ON cdr_final_recovery BEGIN SELECT RAISE(ABORT,'grant failed'); END").unwrap();
    assert!(authorize(&path, &r, render).is_err());
    assert_eq!(
        crate::delivery::list_pending(&path).unwrap()[0].content,
        "Saved answer"
    );
    db.execute_batch("DROP TRIGGER fail_grant").unwrap();
    authorize(&path, &r, render).unwrap();
    let pending = crate::delivery::list_pending(&path).unwrap().remove(0);
    assert!(authorized(&path, &pending).unwrap());
    db.execute("UPDATE discord_ingress_journal SET owner_user_id=4", [])
        .unwrap();
    assert!(claim(&path, &r, 0, &sha256(&pending.content)).is_err());
    assert_eq!(receipt::unknown_count(&path).unwrap(), 0);
}
#[test]
fn concurrent_claims_produce_one_intent_and_partial_delivery_only_claims_remaining_chunks() {
    let (_dir, path, r) = fixture();
    authorize(&path, &r, |s| {
        vec![
            s[..EXPLANATION.len()].to_owned(),
            s[EXPLANATION.len()..].to_owned(),
        ]
    })
    .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let joins = (0..2)
        .map(|_| {
            let path = path.clone();
            let r = r.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                claim(&path, &r, 0, &sha256(EXPLANATION)).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = joins
        .into_iter()
        .map(|j| j.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|s| **s == receipt::ReceiptState::New)
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|s| **s == receipt::ReceiptState::Unknown)
            .count(),
        1
    );
    let key = json!([42, "completion/v1", r.delivery_id, 0]).to_string();
    receipt::confirm(&path, &key, "456").unwrap();
    assert!(matches!(
        claim(&path, &r, 0, &sha256(EXPLANATION)).unwrap(),
        receipt::ReceiptState::Delivered(_)
    ));
    assert_eq!(
        claim(&path, &r, 1, &sha256("Saved answer")).unwrap(),
        receipt::ReceiptState::New
    );
    assert_eq!(
        claim(&path, &r, 1, &sha256("Saved answer")).unwrap(),
        receipt::ReceiptState::Unknown
    );
}
