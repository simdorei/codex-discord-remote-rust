//! An unresolved start defers only its own thread and preserves its cursor.
use cdr_runtime::session_mirror_worker::{
    SessionMirrorDeliveryIdentity, SessionMirrorSender, SessionMirrorWorker,
};
use cdr_store::{
    mapping::upsert_thread,
    mirror::{get_offset, update_cursor},
    queue::{
        NewQueueJob, attach_goal_turn, begin_attempt, enqueue, mark_goal_waiting, mark_running,
    },
};
use rusqlite::Connection;
use serde_json::json;
use std::{
    fs,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Sender(Mutex<Vec<(u64, String)>>);
impl SessionMirrorSender for Sender {
    fn send<'a>(
        &'a self,
        channel: u64,
        _: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            self.0.lock().unwrap().push((channel, text.into()));
            Ok(())
        })
    }
}

#[tokio::test]
async fn unresolved_start_holds_own_cursor_without_losing_other_thread_final() {
    assert_unresolved_ownership(PendingOwnership::Start).await;
}

#[tokio::test]
async fn goal_continuation_waiting_for_turn_binding_does_not_escape_through_mirror() {
    assert_unresolved_ownership(PendingOwnership::GoalWaiting).await;
}

#[tokio::test]
async fn observed_terminal_before_goal_waiting_blocks_next_turn_mirror_escape() {
    assert_unresolved_ownership(PendingOwnership::TerminalObserved).await;
}

#[derive(PartialEq)]
enum PendingOwnership {
    Start,
    GoalWaiting,
    TerminalObserved,
}

async fn assert_unresolved_ownership(scenario: PendingOwnership) {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let mirror = temp.path().join("mirror.sqlite");
    let connection = Connection::open(&state).unwrap();
    connection.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT);").unwrap();
    for (thread, channel) in [("owned", 201_i64), ("other", 202)] {
        let rollout = temp.path().join(format!("{thread}.jsonl"));
        fs::write(
            &rollout,
            format!(
                "{}\n",
                json!({"timestamp":"1", "type":"event_msg",
            "payload":{"type":"task_complete", "turn_id":format!("{thread}-turn"),
                "last_agent_message":format!("{thread} final")}})
            ),
        )
        .unwrap();
        connection.execute("INSERT INTO threads VALUES (?1,'Title','C:/repo',1,?2,'gpt','high',0,0,0,'vscode','user')",
            rusqlite::params![thread, rollout.to_string_lossy()]).unwrap();
        upsert_thread(&mirror, thread, "project", "Title", 100, channel, 1.0).unwrap();
        update_cursor(&mirror, thread, &rollout.to_string_lossy(), 0, 1.0).unwrap();
    }
    enqueue(
        &mirror,
        NewQueueJob {
            job_id: "starting",
            target_thread_id: "owned",
            channel_id: 201,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: 7,
            prompt: "input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_attempt(&mirror, "starting", &[], 7).unwrap();
    if scenario != PendingOwnership::Start {
        mark_running(&mirror, "starting", "previous-turn", 7).unwrap();
    }
    if scenario == PendingOwnership::TerminalObserved {
        assert!(
            cdr_store::observed_completion::record(
                &mirror,
                "owned",
                "previous-turn",
                7,
                r#"{"threadId":"owned","turn":{"id":"previous-turn","status":"completed"}}"#
            )
            .unwrap()
        );
    }
    if scenario == PendingOwnership::GoalWaiting {
        assert!(mark_goal_waiting(&mirror, "starting", "previous-turn", 7).unwrap());
    }
    let sender = Arc::new(Sender::default());
    let worker = SessionMirrorWorker::new(state, mirror.clone(), Arc::clone(&sender));
    assert!(
        worker.poll_once().await.is_err(),
        "unresolved ownership must be reported"
    );
    assert_eq!(get_offset(&mirror, "owned").unwrap().unwrap().cursor, 0);
    assert!(get_offset(&mirror, "other").unwrap().unwrap().cursor > 0);
    assert_eq!(sender.0.lock().unwrap().len(), 1);
    assert_eq!(sender.0.lock().unwrap()[0].0, 202);
    assert!(sender.0.lock().unwrap()[0].1.contains("other final"));
    if scenario == PendingOwnership::TerminalObserved {
        assert!(mark_goal_waiting(&mirror, "starting", "previous-turn", 7).unwrap());
        cdr_store::observed_completion::finish(&mirror, "owned", "previous-turn").unwrap();
    }
    if scenario == PendingOwnership::Start {
        mark_running(&mirror, "starting", "owned-turn", 7).unwrap();
    } else {
        assert!(attach_goal_turn(&mirror, "owned", "owned-turn", 7).unwrap());
    }
    worker.poll_once().await.unwrap();
    assert!(get_offset(&mirror, "owned").unwrap().unwrap().cursor > 0);
    assert_eq!(
        sender.0.lock().unwrap().len(),
        1,
        "runtime-owned final belongs to completion delivery"
    );
}
