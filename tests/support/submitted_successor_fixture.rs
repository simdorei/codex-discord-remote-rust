use cdr_store::{async_question as aq, async_resolution, queue, schema::open_initialized};
use rusqlite::types::Value;
use std::path::Path;

#[path = "async_orphan_fixture.rs"]
pub(crate) mod original;
pub use original::pending;

pub fn submitted(db: &Path, resident: &str) -> String {
    let id = original::dispatching(db, resident);
    aq::confirm_dispatch(db, &id, "original").unwrap();
    assert_eq!(aq::get(db, &id).unwrap().state, "submitted");
    id
}

pub fn observe(db: &Path, resident: &str, turn: &str) {
    let payload = serde_json::json!({
        "threadId": "thread-b", "turn": {"id": turn, "status": "completed"}
    })
    .to_string();
    async_resolution::record_terminal_notification(db, "thread-b", turn, 1, resident, &payload)
        .unwrap();
}

pub fn successor(db: &Path, resident: &str) -> String {
    let id = submitted(db, resident);
    observe(db, resident, "original");
    assert!(queue::mark_goal_waiting(db, "origin", "original", 1).unwrap());
    let waiting = queue::list(db).unwrap().remove(0);
    assert!(queue::attach_goal_turn_observed_if_owned(db, &waiting, "successor", 1).unwrap());
    aq::validate_dispatch_guards(db, "thread-b").unwrap();
    id
}

// Compare original question/seal/choice, execution proof, ownership and fences,
// not just a presentation field. This helper only reads isolated fixture DBs.
pub fn snapshot(db: &Path) -> Vec<Vec<Vec<Value>>> {
    let db = open_initialized(db).unwrap();
    [
        "SELECT * FROM cdr_async_questions ORDER BY id",
        "SELECT * FROM codex_turn_queue ORDER BY job_id",
        "SELECT * FROM cdr_async_execution_obligations ORDER BY question_id",
        "SELECT * FROM cdr_async_execution_handoffs ORDER BY question_id,revision",
        "SELECT * FROM mirror_threads ORDER BY codex_thread_id",
        "SELECT * FROM codex_archive_fences ORDER BY target_thread_id",
        "SELECT * FROM cdr_cleanup_fences ORDER BY channel_id",
        "SELECT * FROM codex_dead_generation_holds ORDER BY target_thread_id",
    ]
    .into_iter()
    .map(|sql| {
        let mut statement = db.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|column| row.get::<_, Value>(column))
                    .collect()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}
