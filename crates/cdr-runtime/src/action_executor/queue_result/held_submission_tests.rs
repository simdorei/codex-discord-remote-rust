//! PATCH-02: current target holds must reach queue and action presentations.
use crate::queue_runner::{
    BackendFailure, BackendFailureKind, BoxBackendFuture, QueueCoordinator, Submission,
    TurnBackend, TurnRecord,
};
use cdr_store::{async_question, delivery, queue, schema::open_initialized};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::async_orphan_tests::submitted_fixture as fixture;

struct Backend {
    generation: u64,
    resume_error: Option<BackendFailure>,
    resumes: AtomicUsize,
    starts: AtomicUsize,
}

impl Backend {
    fn new(generation: u64) -> Arc<Self> {
        Arc::new(Self {
            generation,
            resume_error: None,
            resumes: 0.into(),
            starts: 0.into(),
        })
    }

    fn counts(&self) -> (usize, usize) {
        (
            self.resumes.load(Ordering::SeqCst),
            self.starts.load(Ordering::SeqCst),
        )
    }
}

impl TurnBackend for Backend {
    fn generation(&self) -> u64 {
        self.generation
    }
    fn active_turn_id<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }
    fn read_turns<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn resume_thread<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.resumes.fetch_add(1, Ordering::SeqCst);
            self.resume_error.clone().map_or(Ok(()), Err)
        })
    }
    fn start_turn<'a>(&'a self, _: &'a str, _: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok("new-turn".into())
        })
    }
}

fn orphan(db: &Path) {
    fixture::submitted(db, "old-resident");
    queue::complete(db, "origin").unwrap();
    assert!(async_question::target_dispatch_held(db, "thread-b").unwrap());
}

fn assert_held(submission: &Submission) {
    assert_eq!(
        submission.warning.as_ref().map(|warning| warning.kind),
        Some(BackendFailureKind::ExecutionHeld),
        "{submission:?}"
    );
    let shown = super::submission_result("thread-b", Some("mirror"), submission, "new input");
    assert!(!shown.waits_for_final, "{}", shown.text);
    assert!(shown.text.contains("no automatic replay"), "{}", shown.text);
    assert!(
        !shown.text.contains("queued for automatic retry"),
        "{}",
        shown.text
    );
    assert!(!shown.text.starts_with("Queued\n"), "{}", shown.text);
}

#[tokio::test]
async fn early_admission_reports_current_hold_without_attempting() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    orphan(&db);
    let mut before = fixture::snapshot(&db);
    let backend = Backend::new(2);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    let submitted = coordinator
        .submit_identified("next", "thread-b", 20, 30, Some(50), "new input")
        .await
        .unwrap();
    assert_held(&submitted);
    let jobs = queue::list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, queue::QueueJobState::Pending);
    assert_eq!(jobs[0].attempt_count, 0);
    assert_eq!(jobs[0].prompt, "new input");
    assert!(jobs[0].last_error.is_empty());
    assert!(jobs[0].baseline_turn_ids.is_empty());
    assert_eq!(backend.counts(), (0, 0));
    let mut after = fixture::snapshot(&db);
    before[1].clear();
    after[1].clear();
    assert_eq!(
        after, before,
        "admission must not rewrite the prior obligation"
    );
}

#[tokio::test]
async fn generation_deferred_admission_still_reports_current_hold() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::successor(&db, "old-resident");
    let origin = queue::list(&db).unwrap().remove(0);
    let backend = Backend::new(2);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    let submitted = coordinator
        .submit_identified("next", "thread-b", 20, 30, Some(50), "new input")
        .await
        .unwrap();
    assert_held(&submitted);
    assert_eq!(
        queue::list(&db)
            .unwrap()
            .into_iter()
            .find(|job| job.job_id == "origin")
            .unwrap(),
        origin
    );
    assert_eq!(backend.counts(), (0, 0));
}

async fn assert_saved_replay(error: &str) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    orphan(&db);
    fixture::pending(&db, "next", "thread-b", 2);
    open_initialized(&db).unwrap().execute(
        "UPDATE codex_turn_queue SET discord_message_id=50,last_error=?,attempt_count=2,updated_at=1 WHERE job_id='next'",
        [error],
    ).unwrap();
    let before = fixture::snapshot(&db);
    let backend = Backend::new(2);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    assert_held(
        &coordinator
            .replay_submission_for_message(50)
            .unwrap()
            .unwrap(),
    );
    assert_held(
        &coordinator
            .replay_submission_for_job("next")
            .unwrap()
            .unwrap(),
    );
    let (target, submitted) = coordinator
        .replay_submission_with_target_for_job("next")
        .unwrap()
        .unwrap();
    assert_eq!(target, "thread-b");
    assert_held(&submitted);
    assert_held(
        &coordinator
            .submit_identified("next", "thread-b", 20, 30, Some(50), "new input")
            .await
            .unwrap(),
    );
    assert_held(
        &coordinator
            .submit_mirror_identified("next", "thread-b", 20, 30, Some(50), "new input")
            .await
            .unwrap(),
    );
    if !error.is_empty() {
        assert!(submitted.warning.as_ref().unwrap().message.contains(error));
    }
    let cold_backend = Backend::new(3);
    let cold = QueueCoordinator::new(db.clone(), Arc::clone(&cold_backend));
    assert_held(&cold.replay_submission_for_job("next").unwrap().unwrap());
    cold.kick_target("thread-b").await.unwrap();
    assert_eq!(backend.counts(), (0, 0));
    assert_eq!(cold_backend.counts(), (0, 0));
    assert_eq!(fixture::snapshot(&db), before);
}

#[tokio::test]
async fn saved_empty_error_replay_uses_current_target_hold() {
    assert_saved_replay("").await;
}

#[tokio::test]
async fn saved_historical_failure_replay_uses_current_target_hold() {
    assert_saved_replay("idle subscription release: durable queue job not found: missing-origin")
        .await;
}

#[tokio::test]
async fn same_db_action_and_duplicate_do_not_promise_automatic_retry() {
    use crate::action_executor::{ActionContext, ActionExecutor};
    use crate::bridge_state::BridgeState;
    use crate::command_plan::CommandAction;
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    orphan(&db);
    let state = temp.path().join("codex.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("../../../tests/fixtures/action_state.sql"))
        .unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-b")).unwrap();
    let backend = Backend::new(2);
    let coordinator = Arc::new(QueueCoordinator::new(db.clone(), Arc::clone(&backend)));
    let executor = ActionExecutor::new(state, db.clone(), bridge, coordinator);
    for _ in 0..2 {
        let result = executor
            .execute_with_context(
                CommandAction::Ask {
                    prompt: "new input".into(),
                },
                ActionContext {
                    channel_id: 20,
                    user_id: 30,
                    discord_message_id: Some(50),
                    auto_queue_when_busy: true,
                },
            )
            .await
            .unwrap();
        assert!(!result.waits_for_final, "{}", result.text);
        assert!(
            result.text.contains("no automatic replay"),
            "{}",
            result.text
        );
        assert!(
            !result.text.contains("queued for automatic retry"),
            "{}",
            result.text
        );
    }
    let jobs = queue::list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].attempt_count, 0);
    assert!(jobs[0].last_error.is_empty());
    assert!(
        cdr_store::prompt_intake::list_prompt_intakes(&db)
            .unwrap()
            .is_empty()
    );
    assert_eq!(backend.counts(), (0, 0));
}

#[tokio::test]
async fn verified_settlement_removes_projection_and_starts_only_pending() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::successor(&db, "resident");
    fixture::pending(&db, "next", "thread-b", 2);
    let before = fixture::snapshot(&db);
    let backend = Backend::new(2);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    assert_held(
        &coordinator
            .replay_submission_for_job("next")
            .unwrap()
            .unwrap(),
    );
    assert_eq!(fixture::snapshot(&db), before);

    fixture::observe(&db, "resident", "successor");
    let current = queue::list(&db)
        .unwrap()
        .into_iter()
        .find(|job| job.job_id == "origin")
        .unwrap();
    delivery::stage_owned_queue_completion_with_release(
        &db,
        &current,
        "Final",
        6.0,
        Some(("resident", 1)),
    )
    .unwrap();
    let cleared = coordinator
        .replay_submission_for_job("next")
        .unwrap()
        .unwrap();
    assert!(cleared.warning.is_none());
    assert!(super::submission_result("thread-b", None, &cleared, "new input").waits_for_final);
    coordinator.kick_target("thread-b").await.unwrap();
    let jobs = queue::list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "next");
    assert_eq!(jobs[0].attempt_count, 1);
    assert_eq!(jobs[0].turn_id.as_deref(), Some("new-turn"));
    assert_eq!(backend.counts(), (1, 1));
}

#[tokio::test]
async fn unrelated_transient_failure_keeps_automatic_retry_presentation() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    orphan(&db);
    let backend = Arc::new(Backend {
        generation: 2,
        resume_error: Some(BackendFailure::definite("temporary backend unavailable")),
        resumes: 0.into(),
        starts: 0.into(),
    });
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    let submitted = coordinator
        .submit_identified("other", "unrelated", 21, 30, Some(51), "other input")
        .await
        .unwrap();
    assert_eq!(
        submitted.warning.as_ref().unwrap().kind,
        BackendFailureKind::Other
    );
    let shown = super::submission_result("unrelated", None, &submitted, "other input");
    assert!(shown.waits_for_final);
    assert!(shown.text.contains("queued for automatic retry"));
    let before = queue::list(&db).unwrap();
    assert_eq!(
        coordinator
            .replay_submission_for_job("other")
            .unwrap()
            .unwrap(),
        submitted
    );
    assert_eq!(queue::list(&db).unwrap(), before);
    assert_eq!(backend.counts(), (1, 0));
}

fn assert_starting_outcome_is_visible(error: &str) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::successor(&db, "resident");
    fixture::pending(&db, "unknown", "unrelated", 2);
    queue::begin_attempt(&db, "unknown", &[], 2).unwrap();
    open_initialized(&db).unwrap().execute(
        "UPDATE codex_turn_queue SET target_thread_id='thread-b',discord_message_id=50,last_error=? WHERE job_id='unknown'",
        [error],
    ).unwrap();
    fixture::pending(&db, "next", "thread-b", 2);
    let before = fixture::snapshot(&db);
    let backend = Backend::new(2);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    let unknown = coordinator
        .replay_submission_for_message(50)
        .unwrap()
        .unwrap();
    assert_held(&unknown);
    assert!(unknown.warning.as_ref().unwrap().ambiguous);
    let shown = super::submission_result("thread-b", Some("mirror"), &unknown, "new input");
    assert!(
        shown.text.contains("may already have reached Codex"),
        "{}",
        shown.text
    );
    assert!(
        shown.text.contains("outcome remains unknown"),
        "{}",
        shown.text
    );
    if !error.is_empty() {
        assert!(shown.text.contains(error), "{}", shown.text);
    }
    let pending = coordinator
        .replay_submission_for_job("next")
        .unwrap()
        .unwrap();
    assert_held(&pending);
    assert!(!pending.warning.as_ref().unwrap().ambiguous);
    let shown = super::submission_result("thread-b", Some("mirror"), &pending, "new input");
    assert!(
        !shown.text.contains("may already have reached Codex"),
        "{}",
        shown.text
    );
    assert_eq!(fixture::snapshot(&db), before);
    assert_eq!(backend.counts(), (0, 0));
}

#[test]
fn review_starting_without_error_keeps_visible_unknown_outcome() {
    assert_starting_outcome_is_visible("");
}

#[test]
fn review_starting_transport_error_keeps_visible_unknown_outcome() {
    assert_starting_outcome_is_visible("transport closed");
}

#[tokio::test]
async fn review_terminal_policy_hold_does_not_claim_unconfirmed_execution() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::successor(&db, "resident");
    fixture::pending(&db, "next", "thread-b", 2);
    // Represent an existing protected policy in this isolated fixture only.
    // The actual terminal proof and settlement still use production APIs.
    open_initialized(&db).unwrap().execute(
        "UPDATE cdr_async_execution_obligations SET policy='publishing_recovery' WHERE question_id=?",
        [&id],
    ).unwrap();
    fixture::observe(&db, "resident", "successor");
    let current = queue::list(&db)
        .unwrap()
        .into_iter()
        .find(|job| job.job_id == "origin")
        .unwrap();
    delivery::stage_owned_queue_completion_with_release(
        &db,
        &current,
        "Final",
        6.0,
        Some(("resident", 1)),
    )
    .unwrap();
    let connection = open_initialized(&db).unwrap();
    let terminal_with_policy_hold: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_execution_obligations o
         JOIN cdr_async_terminal_settlements s ON s.question_id=o.question_id
         WHERE o.question_id=? AND o.execution_state='terminal' AND o.admission_state='held'
         AND o.policy='publishing_recovery' AND s.revision=o.revision AND s.proof_json=o.terminal_proof_json)",
        [&id], |row| row.get(0),
    ).unwrap();
    assert!(
        terminal_with_policy_hold,
        "fixture must have a real exact terminal certificate"
    );
    let certificate: (i64, String) = connection
        .query_row(
            "SELECT revision,proof_json FROM cdr_async_terminal_settlements WHERE question_id=?",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(async_question::target_dispatch_held(&db, "thread-b").unwrap());
    let before = fixture::snapshot(&db);
    let backend = Backend::new(2);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    let pending = coordinator
        .replay_submission_for_job("next")
        .unwrap()
        .unwrap();
    assert_held(&pending);
    let shown = super::submission_result("thread-b", Some("mirror"), &pending, "new input");
    assert!(
        !shown.text.contains("prior async execution is unresolved"),
        "{}",
        shown.text
    );
    assert!(
        shown.text.contains("recovery authorization"),
        "{}",
        shown.text
    );
    assert!(
        !shown.text.contains("may already have reached Codex"),
        "{}",
        shown.text
    );
    coordinator.kick_target("thread-b").await.unwrap();
    assert_eq!(fixture::snapshot(&db), before);
    let after: (i64, String) = connection
        .query_row(
            "SELECT revision,proof_json FROM cdr_async_terminal_settlements WHERE question_id=?",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, certificate);
    assert_eq!(backend.counts(), (0, 0));
}

#[tokio::test]
async fn unknown_start_and_existing_strong_holds_preserve_their_authority() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::successor(&db, "resident");
    let backend = Backend::new(2);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    let running = coordinator
        .replay_submission_for_job("origin")
        .unwrap()
        .unwrap();
    assert_eq!(running.turn_id.as_deref(), Some("successor"));
    assert!(running.warning.is_none());

    fixture::pending(&db, "unknown", "unrelated", 2);
    queue::begin_attempt(&db, "unknown", &[], 2).unwrap();
    open_initialized(&db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET target_thread_id='thread-b' WHERE job_id='unknown'",
            [],
        )
        .unwrap();
    let before = fixture::snapshot(&db);
    let unknown = coordinator
        .replay_submission_for_job("unknown")
        .unwrap()
        .unwrap();
    assert_held(&unknown);
    assert!(
        unknown.warning.unwrap().ambiguous,
        "Starting may already have dispatched"
    );
    assert_eq!(fixture::snapshot(&db), before);

    for (index, (prefix, expected)) in [
        (
            queue::UNRESOLVED_FORK_ERROR_PREFIX,
            BackendFailureKind::ForkFenced,
        ),
        (
            queue::STARTING_CANDIDATE_HOLD_PREFIX,
            BackendFailureKind::StartingCandidatesHeld,
        ),
        (
            cdr_store::execution_hold::PREFIX,
            BackendFailureKind::ExecutionHeld,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("protected-{index}");
        fixture::pending(&db, &id, "thread-b", 2);
        open_initialized(&db)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET last_error=? WHERE job_id=?",
                rusqlite::params![format!("{prefix}existing protected reason"), id],
            )
            .unwrap();
        let before = fixture::snapshot(&db);
        let saved = coordinator.replay_submission_for_job(&id).unwrap().unwrap();
        assert_eq!(saved.warning.unwrap().kind, expected);
        assert_eq!(fixture::snapshot(&db), before);
    }
    assert_eq!(backend.counts(), (0, 0));
}
