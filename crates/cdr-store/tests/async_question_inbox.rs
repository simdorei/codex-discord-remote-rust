//! Candidate identity is durable data, not permission to adopt a later job.
use cdr_store::{async_question as aq, queue, schema::open_initialized};

fn setup() -> (tempfile::TempDir, std::path::PathBuf, aq::QuestionBody) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.sqlite");
    enqueue(&db, "origin", "T1");
    let body = aq::QuestionBody {
        index: 0,
        source_text: "조건을 읽고 선택하세요".into(),
        title: "계속?".into(),
        options: vec!["허용".into(), "보류".into()],
    };
    (dir, db, body)
}

fn enqueue(db: &std::path::Path, job: &str, turn: &str) {
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: job,
            target_thread_id: "thread",
            channel_id: 20,
            owner_user_id: Some(30),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "goal",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, job, &[], 1).unwrap();
    queue::mark_running(db, job, turn, 1).unwrap();
}

fn journal(db: &std::path::Path, body: &aq::QuestionBody) -> String {
    aq::record_observation(
        db,
        &aq::NewQuestion {
            runtime_id: "resident-a",
            generation: 1,
            thread_id: "thread",
            turn_id: "T2",
            item_id: "question-call",
            body,
            now: 2.0,
        },
    )
    .unwrap();
    aq::occurrence_id("thread", "T2", "question-call", 0).unwrap()
}

#[test]
fn unbound_question_survives_reopen_until_exact_goal_job_handoff() {
    let (_dir, db, body) = setup();
    let id = journal(&db, &body); // Even before goal_waiting is processed.
    assert!(aq::get(&db, &id).is_err());
    assert_eq!(aq::reconcile_observations(&db, "resident-a", 1).unwrap(), 0);
    let persisted: String = open_initialized(&db)
        .unwrap()
        .query_row(
            "SELECT body FROM cdr_async_question_inbox WHERE id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<aq::QuestionBody>(&persisted).unwrap(),
        body
    );
    queue::mark_goal_waiting(&db, "origin", "T1", 1).unwrap();
    assert_eq!(aq::reconcile_observations(&db, "resident-a", 1).unwrap(), 0);
    assert!(queue::attach_goal_turn(&db, "thread", "T2", 1).unwrap());
    assert_eq!(aq::reconcile_observations(&db, "resident-a", 1).unwrap(), 1);
    let q = aq::get(&db, &id).unwrap();
    assert_eq!(q.origin_job_id, "origin");
    assert_eq!(q.turn_id, "T2");
    assert_eq!(q.body, body);
    assert!(aq::owner_confirmed(&db, &q).unwrap());
    assert_eq!(journal(&db, &body), id);
    assert_eq!(aq::reconcile_observations(&db, "resident-a", 1).unwrap(), 0);
}

#[test]
fn another_job_same_turn_or_generation_cannot_adopt_unbound_question() {
    for wrong in ["job", "turn", "generation", "actor"] {
        let (_dir, db, body) = setup();
        let id = journal(&db, &body);
        match wrong {
            "job" => {
                queue::complete(&db, "origin").unwrap();
                enqueue(&db, "different", "T2");
            }
            "turn" => {
                queue::mark_goal_waiting(&db, "origin", "T1", 1).unwrap();
                queue::attach_goal_turn(&db, "thread", "different", 1).unwrap();
            }
            "generation" => {
                open_initialized(&db)
                    .unwrap()
                    .execute(
                        "UPDATE codex_turn_queue SET turn_id='T2',app_server_generation=2",
                        [],
                    )
                    .unwrap();
            }
            _ => {
                open_initialized(&db)
                    .unwrap()
                    .execute(
                        "UPDATE codex_turn_queue SET turn_id='T2',owner_user_id=31",
                        [],
                    )
                    .unwrap();
            }
        }
        assert_eq!(
            aq::reconcile_observations(&db, "resident-a", 1).unwrap(),
            0,
            "{wrong}"
        );
        assert!(aq::get(&db, &id).is_err(), "{wrong}");
    }
}

#[test]
fn changed_owner_expires_unbound_data_without_replaying_or_rebinding() {
    let (_dir, db, body) = setup();
    let id = journal(&db, &body);
    aq::retire_old_owner(&db, "resident-b", 1).unwrap();
    queue::mark_goal_waiting(&db, "origin", "T1", 1).unwrap();
    queue::attach_goal_turn(&db, "thread", "T2", 1).unwrap();
    assert_eq!(journal(&db, &body), id);
    assert_eq!(aq::reconcile_observations(&db, "resident-a", 1).unwrap(), 0);
    assert_eq!(aq::reconcile_observations(&db, "resident-b", 1).unwrap(), 0);
    assert!(aq::get(&db, &id).is_err());
    let state: String = open_initialized(&db)
        .unwrap()
        .query_row(
            "SELECT state FROM cdr_async_question_inbox WHERE id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "expired");
}

#[test]
fn unbound_question_protects_room_and_never_changes_its_body() {
    let (_dir, db, mut body) = setup();
    journal(&db, &body);
    queue::complete(&db, "origin").unwrap();
    assert_eq!(
        cdr_store::room_cleanup::pending_reason(&db, 20, Some("thread")).unwrap(),
        Some("unbound async question awaiting original ownership")
    );
    body.source_text = "changed".into();
    assert!(
        aq::record_observation(
            &db,
            &aq::NewQuestion {
                runtime_id: "resident-a",
                generation: 1,
                thread_id: "thread",
                turn_id: "T2",
                item_id: "question-call",
                body: &body,
                now: 3.0
            }
        )
        .unwrap_err()
        .to_string()
        .contains("changed its content")
    );
}
