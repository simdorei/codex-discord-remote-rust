use cdr_store::{delivery, idle_release as idle, queue};
use std::path::Path;

fn enqueue(path: &Path, job: &str, thread: &str) -> cdr_store::Result<queue::QueueEnqueueResult> {
    queue::enqueue(
        path,
        queue::NewQueueJob {
            job_id: job,
            target_thread_id: thread,
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
}

fn completed(path: &Path, job: &str, thread: &str) -> idle::Intent {
    enqueue(path, job, thread).unwrap();
    queue::begin_attempt(path, job, &[], 1).unwrap();
    queue::mark_running(path, job, "T1", 1).unwrap();
    delivery::stage_queue_completion_with_release(path, job, "Final", 2.0, Some(("resident", 1)))
        .unwrap();
    idle::get(path, thread).unwrap().unwrap()
}

#[test]
fn ir2_enqueue_cancels_only_unsent_candidate_and_stale_worker_cannot_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let old = completed(&path, "j1", "A");
    enqueue(&path, "j2", "A").unwrap();
    let current = idle::get(&path, "A").unwrap().unwrap();
    assert_eq!(
        current.state, "Settled",
        "IR2: queued successor must cancel the unsent candidate"
    );
    assert_eq!(current.detail, "CancelledBeforeSend");
    assert!(idle::transition(&path, &old, "Dispatching", "permission").is_err());
}

#[test]
fn ir4_ir5_ir9_unknown_never_settles_by_read_or_new_instance_generation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let old = completed(&path, "j1", "A");
    let sent = idle::transition(&path, &old, "Dispatching", "sent").unwrap();
    let uncertain = idle::transition(&path, &sent, "Unknown", "response lost").unwrap();
    assert!(idle::transition(&path, &uncertain, "Settled", "UnloadedConfirmed").is_err());
    assert!(idle::before_mutation(&path, "new-resident", 1, "A").is_err());
    assert!(idle::before_mutation(&path, "resident", 2, "A").is_err());
    assert!(idle::transition(&path, &old, "Dispatching", "late worker").is_err());
    idle::settle_exited_owner(&path, "wrong", 1).unwrap();
    assert_eq!(idle::get(&path, "A").unwrap().unwrap().state, "Unknown");
    idle::settle_exited_owner(&path, "resident", 1).unwrap();
    assert_eq!(
        idle::get(&path, "A").unwrap().unwrap().detail,
        "OldServerExited"
    );
}

#[test]
fn ir12_resubscribing_is_durable_before_rpc_and_old_ack_cannot_replay() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let candidate = completed(&path, "job", "A");
    let sent = idle::transition(&path, &candidate, "Dispatching", "permission").unwrap();
    let ack = idle::transition(&path, &sent, "AwaitUnload", "ack").unwrap();
    let resuming = idle::before_mutation(&path, "resident", 1, "A")
        .unwrap()
        .unwrap();
    assert_eq!(resuming.state, "Resubscribing");
    assert!(resuming.revision > ack.revision);
    assert!(idle::before_mutation(&path, "resident", 1, "A").is_err());
    assert!(idle::before_mutation(&path, "new", 1, "A").is_err());
    assert!(idle::transition(&path, &ack, "Settled", "UnloadedConfirmed").is_err());
    assert_eq!(
        idle::get(&path, "A").unwrap().unwrap().state,
        "Resubscribing"
    );
}

#[test]
fn ir11_capacity_defers_release_without_losing_final_and_errors_are_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    for i in 0..idle::MAX_UNRESOLVED {
        completed(&path, &format!("j{i}"), &format!("t{i}"));
    }
    enqueue(&path, "last", "other").unwrap();
    queue::begin_attempt(&path, "last", &[], 1).unwrap();
    queue::mark_running(&path, "last", "T1", 1).unwrap();
    delivery::stage_queue_completion_with_release(
        &path,
        "last",
        "kept",
        3.0,
        Some(("resident", 1)),
    )
    .unwrap();
    assert!(idle::get(&path, "other").unwrap().is_none());
    assert!(queue::list(&path).unwrap().is_empty());
    assert_eq!(
        delivery::list_pending(&path).unwrap().len(),
        usize::try_from(idle::MAX_UNRESOLVED).unwrap() + 1
    );
    let first = idle::get(&path, "t0").unwrap().unwrap();
    let bounded = idle::transition(&path, &first, "Candidate", &"가".repeat(2000)).unwrap();
    assert_eq!(bounded.detail.chars().count(), idle::MAX_DIAGNOSTIC_CHARS);
}

#[test]
fn ir3_all_unresolved_question_inbox_and_queue_states_prevent_idle() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    completed(&path, "job", "A");
    let db = cdr_store::schema::open_initialized(&path).unwrap();
    db.execute("INSERT INTO cdr_async_questions(id,runtime_id,generation,thread_id,turn_id,item_id,origin_job_id,channel_id,owner_user_id,body,state,created_at,updated_at)
        VALUES('q','resident',1,'A','T1','item','job',42,1,'{}','observed',1,1)",[]).unwrap();
    for state in ["observed", "open", "dispatching", "unsupported", "unknown"] {
        db.execute("UPDATE cdr_async_questions SET state=?", [state])
            .unwrap();
        assert!(!idle::bot_idle(&path, "A").unwrap(), "question {state}");
    }
    db.execute("UPDATE cdr_async_questions SET state='submitted'", [])
        .unwrap();
    assert!(idle::bot_idle(&path, "A").unwrap());
    db.execute("INSERT INTO cdr_async_question_inbox(id,runtime_id,generation,thread_id,turn_id,item_id,candidate_job_id,candidate_channel_id,candidate_owner_id,body,created_at)
        VALUES('waiting','resident',1,'A','T1','item','job',42,1,'{}',1)",[]).unwrap();
    assert!(!idle::bot_idle(&path, "A").unwrap());
    db.execute("DELETE FROM cdr_async_question_inbox", [])
        .unwrap();
    enqueue(&path, "waiting-job", "A").unwrap();
    for state in ["pending", "starting", "running", "quarantined"] {
        db.execute("UPDATE codex_turn_queue SET state=?", [state])
            .unwrap();
        assert!(!idle::bot_idle(&path, "A").unwrap(), "queue {state}");
    }
}
