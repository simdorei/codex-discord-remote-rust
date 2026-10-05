use super::*;
mod control_disposition;
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::path::Path;

mod discovery;
mod review_revision2;
mod settlement;
mod startup_policy;

fn answer(option: usize) -> String {
    let data = json!({"thread_id":"thread-b","original_turn_id":"original",
        "question_item_id":"question-call","question_index":0,"question_title":"Continue?",
        "selected_option_index":option,"selected_option":if option==0 {"yes"} else {"no"}});
    format!(
        "The user answered exactly this earlier async question through Discord. Apply this selection only to this question; other questions remain unanswered.\n{data}"
    )
}

fn script(status: &str, input: Option<Value>) -> Value {
    json!({"pages":[{"data":[{"id":"original","status":status,
        "items":input.into_iter().collect::<Vec<_>>()}],"nextCursor":null}]})
}

fn user_input(option: usize) -> Value {
    json!({"id":"accepted-answer-input","type":"userMessage",
        "content":[{"type":"text","text":answer(option)}]})
}

struct HistoryFixture {
    temp: tempfile::TempDir,
    db: std::path::PathBuf,
    id: String,
    server: Arc<ResidentAppServer>,
    backend: Arc<crate::app_backend::AppServerTurnBackend>,
}

impl HistoryFixture {
    async fn new(script: &Value) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let script_path = temp.path().join("history.json");
        std::fs::write(&script_path, script.to_string()).unwrap();
        let mut config = crate::test_support::native_fixture::config("async-history");
        config.environment.insert(
            "CDR_ASYNC_HISTORY_LOG".into(),
            temp.path().join("rpc.jsonl").to_string_lossy().into(),
        );
        config.environment.insert(
            "CDR_ASYNC_HISTORY_SCRIPT".into(),
            script_path.to_string_lossy().into(),
        );
        let fence = Arc::new(
            crate::dead_generation_recovery::RuntimeDeadGenerationFence::new(
                db.clone(),
                "history-fixture-runtime".into(),
                Some(42),
            )
            .unwrap(),
        );
        let server = Arc::new(
            ResidentAppServer::start_with_dead_generation_fence(config, fence)
                .await
                .unwrap(),
        );
        let id = fixture::dispatching(&db, server.instance_id());
        aq::record_error(&db, &id, "original send timeout").unwrap();
        crate::idle_release::install(&server, &db).unwrap();
        queue::complete(&db, "origin").unwrap();
        fixture::pending(&db, "next", "thread-b", 1);
        let backend = Arc::new(crate::app_backend::AppServerTurnBackend::new(
            server.clone(),
        ));
        Self {
            temp,
            db,
            id,
            server,
            backend,
        }
    }

    fn queue(&self) -> QueueCoordinator<crate::app_backend::AppServerTurnBackend> {
        QueueCoordinator::new(self.db.clone(), self.backend.clone())
    }

    fn obligation(&self) -> (String, String, String, String, i64) {
        cdr_store::schema::open_initialized(&self.db)
            .unwrap()
            .query_row(
                "SELECT answer_state,execution_state,admission_state,original_error,revision
             FROM cdr_async_execution_obligations WHERE question_id=?",
                [&self.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap()
    }

    fn candidates(&self) -> Vec<Value> {
        cdr_store::schema::open_initialized(&self.db).unwrap().prepare(
            "SELECT evidence_text FROM cdr_async_terminal_candidates WHERE question_id=? ORDER BY evidence_sha256",
        ).unwrap().query_map([&self.id],|r|r.get::<_,String>(0)).unwrap()
            .map(|r|serde_json::from_str(&r.unwrap()).unwrap()).collect()
    }

    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.temp.path().join("rpc.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn assert_no_execution(&self, before: &[queue::StoredQueueJob]) {
        assert_eq!(queue::list(&self.db).unwrap(), before);
        assert!(cdr_store::async_resolution::admission_held(&self.db, "thread-b").unwrap());
        assert_eq!(aq::get(&self.db, &self.id).unwrap().state, "dispatching");
        assert_eq!(
            aq::get(&self.db, &self.id).unwrap().error,
            "original send timeout"
        );
        assert!(
            self.calls().iter().all(|v| matches!(
                v["method"].as_str(),
                Some("initialize" | "thread/read" | "thread/turns/list")
            )),
            "non-read RPC emitted"
        );
        let row = self.obligation();
        assert_eq!(
            (row.1.as_str(), row.2.as_str(), row.3.as_str(), row.4),
            ("unresolved", "held", "original send timeout", 0)
        );
    }
}

#[tokio::test]
async fn exact_orphan_history_is_retained_without_claiming_terminal_authority_or_replaying() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    let report = f.queue().recover_target("thread-b").await.unwrap();
    assert_eq!(f.obligation().0, "exact_history_confirmed");
    assert_eq!(report.started, 0);
    let candidates = f.candidates();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["source"], "historical_read_candidate_v1");
    assert_eq!(candidates[0]["terminal_status"], "completed");
    assert_eq!(candidates[0]["execution_authority"], false);
    assert_eq!(candidates[0]["answer_input_id"], "accepted-answer-input");
    assert_ne!(candidates[0]["source"], "resident_notification_v1");
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn answer_receipt_without_terminal_never_releases_execution() {
    let f = HistoryFixture::new(&script("inProgress", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    f.queue().recover_target("thread-b").await.unwrap();
    assert_eq!(f.obligation().0, "exact_history_confirmed");
    assert!(f.candidates()[0]["terminal_status"].is_null());
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn terminal_without_exact_answer_does_not_invent_a_receipt() {
    let f = HistoryFixture::new(&script("completed", None)).await;
    let before = queue::list(&f.db).unwrap();
    f.queue().recover_target("thread-b").await.unwrap();
    assert_eq!(f.obligation().0, "unresolved");
    let rows = f.candidates();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["terminal_status"], "completed");
    assert!(rows[0]["answer_input_id"].is_null());
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn wrong_target_turn_role_selection_and_incomplete_history_remain_held() {
    for mode in [
        "thread",
        "turn",
        "role",
        "choice",
        "truncated",
        "duplicate-turn",
    ] {
        let mut value = script("completed", Some(user_input(0)));
        match mode {
            "thread" => value["wrong_thread"] = json!("other-thread"),
            "turn" => value["pages"][0]["data"][0]["id"] = json!("other-turn"),
            "role" => value["pages"][0]["data"][0]["items"][0]["type"] = json!("agentMessage"),
            "choice" => value["pages"][0]["data"][0]["items"][0] = user_input(1),
            "truncated" => value["pages"][0]["truncated"] = json!(true),
            "duplicate-turn" => {
                let turn = value["pages"][0]["data"][0].clone();
                value["pages"][0]["data"].as_array_mut().unwrap().push(turn);
            }
            _ => unreachable!(),
        }
        let f = HistoryFixture::new(&value).await;
        let before = queue::list(&f.db).unwrap();
        let _ = f.queue().recover_target("thread-b").await;
        assert!(
            f.calls().iter().any(|v| v["method"] == "thread/read"),
            "no actual read: {mode}"
        );
        assert_eq!(f.obligation().0, "unresolved", "{mode}");
        f.assert_no_execution(&before);
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn repeated_cold_observation_preserves_one_candidate_and_does_not_mint_authority() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    f.queue().recover_target("thread-b").await.unwrap();
    let first = f.candidates();
    assert_eq!(first.len(), 1);
    for _ in 0..2 {
        f.queue().recover_target("thread-b").await.unwrap();
    }
    assert_eq!(f.candidates(), first);
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn endless_history_has_a_bounded_read_budget_and_no_replay() {
    let f = HistoryFixture::new(&json!({"endless":true})).await;
    let before = queue::list(&f.db).unwrap();
    let start = std::time::Instant::now();
    let _ = f.queue().recover_target("thread-b").await;
    assert!(start.elapsed() < Duration::from_secs(12));
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/turns/list")
            .count(),
        8
    );
    assert_eq!(f.obligation().0, "unresolved");
    assert!(f.candidates().is_empty());
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

async fn wait_file(path: &Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "history read did not begin"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn changed_policy_during_history_read_prevents_stale_evidence_commit() {
    let mut value = script("completed", Some(user_input(0)));
    value["gate"] = json!(true);
    let f = HistoryFixture::new(&value).await;
    let before = queue::list(&f.db).unwrap();
    let coordinator = f.queue();
    let task = tokio::spawn(async move { coordinator.recover_target("thread-b").await });
    wait_file(&f.temp.path().join("rpc.jsonl.entered")).await;
    cdr_store::schema::open_initialized(&f.db).unwrap().execute(
        "UPDATE cdr_async_execution_obligations SET policy='publishing_recovery' WHERE question_id=?",
        [&f.id],
    ).unwrap();
    std::fs::write(
        f.temp.path().join("rpc.jsonl.release"),
        b"release exact read",
    )
    .unwrap();
    let _ = task.await.unwrap();
    assert_eq!(f.obligation().0, "unresolved");
    assert!(f.candidates().is_empty());
    let policy: Option<String> = cdr_store::schema::open_initialized(&f.db)
        .unwrap()
        .query_row(
            "SELECT policy FROM cdr_async_execution_obligations WHERE question_id=?",
            [&f.id],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(policy.as_deref(), Some("publishing_recovery"));
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn closed_maintenance_controls_do_not_read_or_commit_history() {
    use crate::restart_readiness::drain::{AdmissionGate, DrainFenceKey};
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    let gate = AdmissionGate::new();
    let key = DrainFenceKey::new("history-test", "1|2", "maintenance").unwrap();
    gate.seal(&key).unwrap();
    gate.close_controls(&key).unwrap();
    let coordinator =
        QueueCoordinator::new_with_admission_gate(f.db.clone(), f.backend.clone(), gate.clone());
    coordinator.recover_target("thread-b").await.unwrap();
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/read")
            .count(),
        0
    );
    assert!(f.candidates().is_empty());
    assert!(gate.is_drained_for(&key));
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn candidate_insert_failure_rolls_back_receipt_state_without_replay() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    cdr_store::schema::open_initialized(&f.db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_history_candidate BEFORE INSERT ON cdr_async_terminal_candidates
         BEGIN SELECT RAISE(ABORT,'injected candidate write failure'); END;",
        )
        .unwrap();
    f.queue().recover_target("thread-b").await.unwrap();
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/turns/list")
            .count(),
        1
    );
    assert_eq!(f.obligation().0, "unresolved");
    assert!(f.candidates().is_empty());
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn full_candidate_budget_does_not_claim_a_receipt_that_was_not_saved() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    for index in 0..8 {
        cdr_store::schema::open_initialized(&f.db)
            .unwrap()
            .execute(
                "INSERT INTO cdr_async_terminal_candidates VALUES(?,0,'unverified',?,?)",
                rusqlite::params![
                    f.id,
                    format!("{index:064x}"),
                    json!({"unverified_noise":index}).to_string()
                ],
            )
            .unwrap();
    }
    let original = f.candidates();
    f.queue().recover_target("thread-b").await.unwrap();
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/turns/list")
            .count(),
        1
    );
    assert_eq!(f.obligation().0, "unresolved");
    assert_eq!(f.candidates(), original);
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn explicit_target_review_works_without_any_remaining_queue_job() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    queue::complete(&f.db, "next").unwrap();
    f.queue().recover_target("thread-b").await.unwrap();
    assert_eq!(f.obligation().0, "exact_history_confirmed");
    assert_eq!(f.candidates().len(), 1);
    f.assert_no_execution(&[]);
    f.server.close().await.unwrap();
}

#[test]
fn known_history_method_is_observational_without_broadening_mutations() {
    assert!(cdr_app_server::requests::is_observational(
        "thread/turns/list"
    ));
    for method in [
        "thread/resume",
        "turn/start",
        "turn/steer",
        "mcpServer/tool/call",
        "thread/turns/unknown",
    ] {
        assert!(
            !cdr_app_server::requests::is_observational(method),
            "{method}"
        );
    }
}

#[tokio::test]
async fn lost_history_response_does_not_quarantine_or_authorize_replay() {
    let f = HistoryFixture::new(&json!({"timeout":true})).await;
    let before = queue::list(&f.db).unwrap();
    let started = std::time::Instant::now();
    f.queue().recover_target("thread-b").await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(12));
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/turns/list")
            .count(),
        1
    );
    assert!(
        !f.server.lifecycle_snapshot().await.quarantined,
        "read-only history timeout quarantined the resident"
    );
    assert!(
        cdr_store::mutation_attempt::check(&f.db, "history-fixture-runtime", Some("thread-b"))
            .is_ok()
    );
    let other = f
        .server
        .execute(
            cdr_app_server::requests::read_thread("thread-a", false),
            Some(1),
        )
        .await
        .unwrap();
    assert_eq!(other["thread"]["id"], "thread-a");
    assert_eq!(f.obligation().0, "unresolved");
    assert!(f.candidates().is_empty());
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}
