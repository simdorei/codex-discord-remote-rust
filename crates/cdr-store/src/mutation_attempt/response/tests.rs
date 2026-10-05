use super::*;
use crate::queue::{self, NewQueueJob};
use serde_json::json;

#[test]
fn changed_relational_response_owner_cannot_commit_admission() {
    for column in ["target_thread_id", "turn_id", "job_id"] {
        let f = Fixture::new();
        let claim = capture(&f.path, &f.scope()).unwrap();
        f.db().execute_batch(&format!(
            "CREATE TRIGGER change_response_owner AFTER INSERT ON cdr_server_responses
             BEGIN UPDATE cdr_server_responses SET {column}='foreign' WHERE request_key=NEW.request_key; END;"
        )).unwrap();
        assert!(
            begin(&f.path, &f.scope(), &claim, &json!({"result":1})).is_err(),
            "changed stored {column} must revoke wire authority"
        );
        assert_eq!(f.count(), 0);
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    path: std::path::PathBuf,
    request: Value,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("response.sqlite");
        super::super::activate(&path, "runtime").unwrap();
        queue::enqueue(
            &path,
            NewQueueJob {
                job_id: "original",
                target_thread_id: "target",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(101),
                app_server_generation: 7,
                prompt: "input",
                queued: false,
                ack_sent: true,
                created_at: 1_790_584_100.000_002_1,
            },
        )
        .unwrap();
        crate::schema::open_initialized(&path)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET state='running',turn_id='turn',attempt_count=1",
                [],
            )
            .unwrap();
        Self {
            _temp: temp,
            path,
            request: json!({"id":71,"occurrence":[1,2,3],
            "method":"item/tool/requestUserInput","params":{"threadId":"target","turnId":"turn"}}),
        }
    }
    fn scope(&self) -> Scope<'_> {
        Scope {
            runtime: "runtime",
            resident: "resident",
            generation: 7,
            request: &self.request,
        }
    }
    fn db(&self) -> Connection {
        crate::schema::open_initialized(&self.path).unwrap()
    }
    fn count(&self) -> i64 {
        self.db()
            .query_row("SELECT count(*) FROM cdr_server_responses", [], |row| {
                row.get(0)
            })
            .unwrap()
    }
}

#[test]
fn stop_after_capture_blocks_admission_without_erasing_original() {
    let f = Fixture::new();
    let claim = capture(&f.path, &f.scope()).unwrap();
    crate::execution_hold::hold_in(&f.db(), "original", "target", "stop", "{}").unwrap();
    assert!(begin(&f.path, &f.scope(), &claim, &json!({"result":1})).is_err());
    assert_eq!(f.count(), 0);
    assert_eq!(queue::list(&f.path).unwrap().len(), 1);
}

#[test]
fn recovery_after_capture_cannot_substitute_a_new_original() {
    let f = Fixture::new();
    let claim = capture(&f.path, &f.scope()).unwrap();
    queue::cancel_for_recovery(&f.path, "target", 42, 3, 2.0).unwrap();
    assert!(begin(&f.path, &f.scope(), &claim, &json!({"result":1})).is_err());
    assert_eq!(f.count(), 0);
    assert!(queue::list(&f.path).unwrap().is_empty());
}

#[test]
fn hold_inserted_by_admission_trigger_rolls_back_both_rows() {
    let f = Fixture::new();
    let claim = capture(&f.path, &f.scope()).unwrap();
    f.db()
        .execute_batch(
            "CREATE TRIGGER stop_in_writer AFTER INSERT ON cdr_server_responses
        BEGIN INSERT INTO cdr_execution_holds VALUES('original','target','stop','{}',1); END;",
        )
        .unwrap();
    assert!(begin(&f.path, &f.scope(), &claim, &json!({"result":1})).is_err());
    assert_eq!(f.count(), 0);
    assert!(
        crate::execution_hold::reason(&f.path, "original")
            .unwrap()
            .is_none()
    );
}

#[test]
fn flush_is_not_terminal_and_occurrence_never_grants_a_second_send() {
    let f = Fixture::new();
    let claim = capture(&f.path, &f.scope()).unwrap();
    let body = json!({"result":1});
    begin(&f.path, &f.scope(), &claim, &body).unwrap();
    assert!(begin(&f.path, &f.scope(), &claim, &body).is_err());
    assert!(finish(&f.path, &f.scope(), &claim, &body, "reply_ok").is_err());
    finish(&f.path, &f.scope(), &claim, &body, "flushed").unwrap();
    let phase: String = f
        .db()
        .query_row("SELECT phase FROM cdr_server_responses", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(phase, "flushed");
    assert!(begin(&f.path, &f.scope(), &claim, &body).is_err());
    assert_eq!(f.count(), 1);
}

#[test]
fn new_runtime_cannot_finish_or_reuse_an_old_response() {
    let f = Fixture::new();
    let claim = capture(&f.path, &f.scope()).unwrap();
    let body = json!({"result":1});
    begin(&f.path, &f.scope(), &claim, &body).unwrap();
    super::super::activate(&f.path, "replacement").unwrap();
    assert!(finish(&f.path, &f.scope(), &claim, &body, "flushed").is_err());
    assert!(begin(&f.path, &f.scope(), &claim, &body).is_err());
    assert_eq!(f.count(), 1);
}

#[test]
fn tampered_response_and_failed_finish_preserve_admitted_evidence() {
    let f = Fixture::new();
    let claim = capture(&f.path, &f.scope()).unwrap();
    let body = json!({"result":1});
    begin(&f.path, &f.scope(), &claim, &body).unwrap();
    assert!(finish(&f.path, &f.scope(), &claim, &json!({"result":2}), "flushed").is_err());
    f.db()
        .execute_batch(
            "CREATE TRIGGER reject_response_finish BEFORE UPDATE ON cdr_server_responses
        BEGIN SELECT RAISE(ABORT,'fixture finish failure'); END;",
        )
        .unwrap();
    assert!(finish(&f.path, &f.scope(), &claim, &body, "flushed").is_err());
    assert!(check(&f.path, "target").is_err());
    assert!(check(&f.path, "other").is_ok());
    assert_eq!(f.count(), 1);
}

#[test]
fn original_owner_and_mapping_cannot_change_after_capture() {
    for change in [
        "UPDATE codex_turn_queue SET owner_user_id=99",
        "INSERT INTO mirror_threads(codex_thread_id,project_key,thread_title,discord_channel_id,discord_thread_id,updated_at) VALUES('other','p','t',100,42,1)",
    ] {
        let f = Fixture::new();
        let claim = capture(&f.path, &f.scope()).unwrap();
        f.db().execute_batch(change).unwrap();
        assert!(begin(&f.path, &f.scope(), &claim, &json!({"result":1})).is_err());
        assert_eq!(f.count(), 0);
    }
}

#[test]
fn writer_first_evidence_outlives_stop_and_only_exact_terminal_settles() {
    let f = Fixture::new();
    let claim = capture(&f.path, &f.scope()).unwrap();
    let body = json!({"result":1});
    begin(&f.path, &f.scope(), &claim, &body).unwrap();
    crate::execution_hold::hold_in(&f.db(), "original", "target", "stop", "{}").unwrap();
    let payload =
        json!({"threadId":"target","turn":{"id":"turn","status":"completed"}}).to_string();
    record_terminal_in(&f.db(), "target", "turn", 7, "foreign", &payload).unwrap();
    assert!(check_all(&f.path).is_err());
    record_terminal_in(&f.db(), "target", "turn", 7, "resident", &payload).unwrap();
    finish(&f.path, &f.scope(), &claim, &body, "flushed").unwrap();
    let phase: String = f
        .db()
        .query_row("SELECT phase FROM cdr_server_responses", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(phase, "terminal");
    assert!(
        crate::execution_hold::reason(&f.path, "original")
            .unwrap()
            .is_some()
    );
    assert!(begin(&f.path, &f.scope(), &claim, &body).is_err());
}

#[test]
fn failed_admission_has_no_send_authority_and_indeterminate_is_not_evicted() {
    let f = Fixture::new();
    let claim = capture(&f.path, &f.scope()).unwrap();
    let body = json!({"result":1});
    f.db()
        .execute_batch(
            "CREATE TRIGGER reject_response BEFORE INSERT ON cdr_server_responses
        BEGIN SELECT RAISE(ABORT,'fixture admission failure'); END;",
        )
        .unwrap();
    assert!(begin(&f.path, &f.scope(), &claim, &body).is_err());
    assert_eq!(f.count(), 0);
    f.db()
        .execute_batch("DROP TRIGGER reject_response")
        .unwrap();
    begin(&f.path, &f.scope(), &claim, &body).unwrap();
    assert!(check_all(&f.path).is_err());
    assert!(check(&f.path, "other").is_ok());
    assert!(capture(&f.path, &f.scope()).is_err());
    assert_eq!(f.count(), 1);
}
