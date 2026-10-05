use super::*;
use sha2::{Digest, Sha256};

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn stored_candidate(f: &HistoryFixture) -> Value {
    let row = cdr_store::async_resolution::inspect(&f.db, "thread-b")
        .unwrap()
        .remove(0);
    let input = user_input(0);
    let facts = json!({"question_id":f.id,"revision":row.revision,
        "claim_sha256":row.claim_sha256,"thread_id":"thread-b","original_turn_id":"original",
        "input":input,"answer_conflict":false,"terminal_status":"completed"});
    json!({"version":1,"source":"historical_read_candidate_v1","observer":"older-reader",
        "generation":0,"question_id":f.id,"revision":row.revision,
        "claim_sha256":row.claim_sha256,"thread_id":"thread-b","original_turn_id":"original",
        "history_sha256":digest("older complete history"),"original_turn_sha256":digest("older original turn"),
        "answer_input_id":"accepted-answer-input","matching_input":input,"answer_conflict":false,
        "terminal_status":"completed","review_key":digest(&facts.to_string()),"execution_authority":false})
}

#[tokio::test]
async fn replaced_candidate_cannot_certify_a_missing_stored_answer() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    cdr_store::schema::open_initialized(&f.db).unwrap().execute_batch(
        "CREATE TRIGGER replace_history_candidate BEFORE INSERT ON cdr_async_terminal_candidates
         WHEN json_type(NEW.evidence_text,'$.matching_input')='object'
         BEGIN
           INSERT INTO cdr_async_terminal_candidates(question_id,revision,kind,evidence_sha256,evidence_text)
           VALUES(NEW.question_id,NEW.revision,NEW.kind,NEW.evidence_sha256,
             json_remove(NEW.evidence_text,'$.matching_input','$.answer_input_id'));
           SELECT RAISE(IGNORE);
         END;",
    ).unwrap();
    f.queue().recover_target("thread-b").await.unwrap();
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/turns/list")
            .count(),
        1
    );
    assert_eq!(
        f.obligation().0,
        "unresolved",
        "receipt advanced without stored exact answer evidence"
    );
    assert!(
        f.candidates().is_empty(),
        "failed evidence transaction must roll back the substituted row"
    );
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn invalid_preexisting_historical_candidate_cannot_certify_receipt() {
    for mode in [
        "digest",
        "source",
        "authority",
        "claim",
        "revision",
        "input",
        "input-id",
        "observer",
        "generation",
        "history-hash",
    ] {
        let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
        let before = queue::list(&f.db).unwrap();
        let mut evidence = stored_candidate(&f);
        match mode {
            "digest" => {}
            "source" => evidence["source"] = json!("resident_notification_v1"),
            "authority" => evidence["execution_authority"] = json!(true),
            "claim" => evidence["claim_sha256"] = json!(digest("different original claim")),
            "revision" => evidence["revision"] = json!(1),
            "input" => evidence["matching_input"] = Value::Null,
            "input-id" => evidence["answer_input_id"] = json!("different-input"),
            "observer" => evidence["observer"] = json!(" "),
            "generation" => evidence["generation"] = json!(-1),
            "history-hash" => evidence["history_sha256"] = json!("not-a-hash"),
            _ => unreachable!(),
        }
        let raw = evidence.to_string();
        let hash = if mode == "digest" {
            digest("different bytes")
        } else {
            digest(&raw)
        };
        cdr_store::schema::open_initialized(&f.db)
            .unwrap()
            .execute(
                "INSERT INTO cdr_async_terminal_candidates VALUES(?,0,'unverified',?,?)",
                rusqlite::params![f.id, hash, raw],
            )
            .unwrap();
        let original = f.candidates();
        f.queue().recover_target("thread-b").await.unwrap();
        assert_eq!(
            f.calls()
                .iter()
                .filter(|v| v["method"] == "thread/turns/list")
                .count(),
            1
        );
        assert_eq!(f.obligation().0, "unresolved", "{mode}");
        assert_eq!(
            f.candidates(),
            original,
            "immutable evidence must not be replaced or quota reset: {mode}"
        );
        f.assert_no_execution(&before);
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn duplicate_item_identity_remains_unresolved_and_distinct_item_is_allowed() {
    for duplicate in [true, false] {
        let mut value = script("completed", Some(user_input(0)));
        value["pages"][0]["data"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id":if duplicate {"accepted-answer-input"} else {"different-user-input"},
                "type":"userMessage","content":[{"type":"text","text":"unrelated normal input"}],
            }));
        let f = HistoryFixture::new(&value).await;
        let before = queue::list(&f.db).unwrap();
        f.queue().recover_target("thread-b").await.unwrap();
        assert_eq!(
            f.calls()
                .iter()
                .filter(|v| v["method"] == "thread/turns/list")
                .count(),
            1
        );
        assert_eq!(
            f.obligation().0,
            if duplicate {
                "unresolved"
            } else {
                "exact_history_confirmed"
            },
            "one item ID must not identify two different inputs"
        );
        f.assert_no_execution(&before);
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn valid_stored_semantics_are_reused_despite_older_reader_and_snapshot_identity() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    let evidence = stored_candidate(&f);
    let raw = evidence.to_string();
    cdr_store::schema::open_initialized(&f.db)
        .unwrap()
        .execute(
            "INSERT INTO cdr_async_terminal_candidates VALUES(?,0,'unverified',?,?)",
            rusqlite::params![f.id, digest(&raw), raw],
        )
        .unwrap();
    f.queue().recover_target("thread-b").await.unwrap();
    assert_eq!(f.obligation().0, "exact_history_confirmed");
    assert_eq!(f.candidates(), vec![evidence]);
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}
