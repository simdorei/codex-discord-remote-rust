//! R2: terminal unsent tombstones are retained without permanently blocking idle.
use cdr_store::{
    async_question as aq, delivery, idle_release as idle, queue, schema::open_initialized,
};
use std::path::PathBuf;

fn fixture(bound: bool) -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    queue::enqueue(
        &db,
        queue::NewQueueJob {
            job_id: "origin",
            target_thread_id: "thread",
            channel_id: 20,
            owner_user_id: Some(30),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(&db, "origin", &[], 1).unwrap();
    queue::mark_running(&db, "origin", "T1", 1).unwrap();
    let turn = if bound { "T1" } else { "T2" };
    aq::observe(
        &db,
        &aq::NewQuestion {
            runtime_id: "resident",
            generation: 1,
            thread_id: "thread",
            turn_id: turn,
            item_id: "item",
            body: &aq::QuestionBody {
                index: 0,
                source_text: String::new(),
                title: "?".into(),
                options: vec!["yes".into()],
            },
            now: 2.0,
        },
    )
    .unwrap();
    let id = aq::occurrence_id("thread", turn, "item", 0).unwrap();
    (temp, db, id)
}

#[test]
fn never_dispatched_expired_question_allows_idle_without_removing_tombstone() {
    for retirement in [false, true] {
        let (_temp, db, id) = fixture(true);
        queue::complete(&db, "origin").unwrap();
        assert!(!idle::bot_idle(&db, "thread").unwrap());
        if retirement {
            aq::retire_old_owner(&db, "new-resident", 2).unwrap();
        } else {
            aq::supersede(&db, "resident", 1, "thread", "T2").unwrap();
        }
        assert_eq!(aq::get(&db, &id).unwrap().state, "expired");
        assert!(idle::bot_idle(&db, "thread").unwrap());
        aq::compact_terminal(&db, 99_999_999_999.0).unwrap();
        assert_eq!(aq::get(&db, &id).unwrap().state, "expired");
        assert!(idle::bot_idle(&db, "thread").unwrap());
    }
}

#[test]
fn expired_inbox_is_not_active_work_but_waiting_and_unknown_still_block() {
    let (_temp, db, id) = fixture(false);
    queue::complete(&db, "origin").unwrap();
    assert!(!idle::bot_idle(&db, "thread").unwrap());
    aq::retire_old_owner(&db, "new-resident", 2).unwrap();
    assert!(idle::bot_idle(&db, "thread").unwrap());
    for state in ["waiting", "unknown", "unexpected"] {
        open_initialized(&db)
            .unwrap()
            .execute("UPDATE cdr_async_question_inbox SET state=?", [state])
            .unwrap();
        assert!(!idle::bot_idle(&db, "thread").unwrap(), "{state}");
    }
    assert!(aq::get(&db, &id).is_err());
}

#[test]
fn expired_label_cannot_hide_any_dispatch_evidence() {
    for column in [
        "chosen=0",
        "dispatch_mode='steer'",
        "reply_job_id='reply'",
        "accepted_turn_id='accepted'",
        "preparation_json='{}'",
    ] {
        let (_temp, db, _id) = fixture(true);
        queue::complete(&db, "origin").unwrap();
        aq::supersede(&db, "resident", 1, "thread", "T2").unwrap();
        open_initialized(&db)
            .unwrap()
            .execute(&format!("UPDATE cdr_async_questions SET {column}"), [])
            .unwrap();
        assert!(!idle::bot_idle(&db, "thread").unwrap(), "{column}");
    }
}

#[test]
fn unsupported_unknown_and_dispatching_questions_remain_idle_blockers() {
    for state in ["observed", "open", "unsupported", "unknown", "dispatching"] {
        let (_temp, db, _id) = fixture(true);
        queue::complete(&db, "origin").unwrap();
        open_initialized(&db)
            .unwrap()
            .execute("UPDATE cdr_async_questions SET state=?", [state])
            .unwrap();
        assert!(!idle::bot_idle(&db, "thread").unwrap(), "{state}");
    }
}

#[test]
fn expired_question_allows_candidate_in_the_next_final_transaction() {
    let (_temp, db, id) = fixture(true);
    aq::supersede(&db, "resident", 1, "thread", "T2").unwrap();
    queue::mark_running(&db, "origin", "T2", 1).unwrap();
    let before = queue::list(&db).unwrap().remove(0);
    delivery::stage_owned_queue_completion_with_release(
        &db,
        &before,
        "Final",
        3.0,
        Some(("resident", 1)),
    )
    .unwrap();
    assert_eq!(
        idle::get(&db, "thread").unwrap().unwrap().state,
        "Candidate"
    );
    assert_eq!(delivery::list_pending(&db).unwrap().len(), 1);
    assert_eq!(aq::get(&db, &id).unwrap().state, "expired");
    assert!(queue::list(&db).unwrap().is_empty());
}
