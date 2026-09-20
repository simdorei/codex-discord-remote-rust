use cdr_store::{
    delivery, idle_release as idle, observed_completion, queue, reserve_policy as reserve,
};
use rusqlite::Connection;
use std::path::Path;

fn running(path: &Path, generation: i64) -> queue::StoredQueueJob {
    queue::enqueue(
        path,
        queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(40),
            app_server_generation: generation,
            prompt: "original",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(path, "job", &[], generation).unwrap();
    queue::mark_running(path, "job", "turn", generation).unwrap();
    observed_completion::record(path, "thread", "turn", generation, "{}").unwrap();
    reserve::ensure(path, "thread").unwrap();
    queue::list(path).unwrap().remove(0)
}

#[test]
fn changed_owner_is_not_consumed_and_its_terminal_journal_survives() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite");
    let expected = running(&path, 1);
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE codex_turn_queue SET owner_user_id=99", [])
        .unwrap();
    let changed = queue::list(&path).unwrap();
    assert!(
        delivery::stage_owned_queue_completion_with_release(
            &path,
            &expected,
            "wrong",
            2.0,
            Some(("resident", 1))
        )
        .is_err()
    );
    assert_eq!(queue::list(&path).unwrap(), changed);
    assert!(delivery::list_pending(&path).unwrap().is_empty());
    assert!(observed_completion::contains(&path, "thread", "turn").unwrap());
    assert!(idle::get(&path, "thread").unwrap().is_none());
}

#[test]
fn failed_candidate_commit_rolls_back_final_and_exact_job_consumption() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite");
    let expected = running(&path, 1);
    Connection::open(&path).unwrap().execute_batch("CREATE TRIGGER fail_candidate BEFORE INSERT ON cdr_idle_release BEGIN SELECT RAISE(ABORT,'injected candidate failure'); END;").unwrap();
    assert!(
        delivery::stage_owned_queue_completion_with_release(
            &path,
            &expected,
            "Final",
            2.0,
            Some(("resident", 1))
        )
        .is_err()
    );
    assert_eq!(queue::list(&path).unwrap(), [expected]);
    assert!(delivery::list_pending(&path).unwrap().is_empty());
    assert!(observed_completion::contains(&path, "thread", "turn").unwrap());
}

#[test]
fn historical_job_final_is_preserved_without_new_release_authority() {
    for authority in [None, Some(("new-resident", 2))] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("db.sqlite");
        let job = running(&path, 1);
        let result = delivery::stage_owned_queue_completion_with_release(
            &path, &job, "Final", 2.0, authority,
        )
        .unwrap();
        assert_eq!(result.content, "Final");
        assert!(queue::list(&path).unwrap().is_empty());
        assert!(idle::get(&path, "thread").unwrap().is_none());
    }
}

#[test]
fn historical_reserve_states_do_not_govern_current_completion_release() {
    for state in [
        "ordinary",
        "reserve",
        "entering",
        "restoring",
        "held",
        "unknown",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("db.sqlite");
        let job = running(&path, 1);
        reserve::ensure(&path, "thread").unwrap();
        Connection::open(&path)
            .unwrap()
            .execute("UPDATE codex_reserve_policy SET state=?", [state])
            .unwrap();
        delivery::stage_owned_queue_completion_with_release(
            &path,
            &job,
            "Final",
            2.0,
            Some(("resident", 1)),
        )
        .unwrap();
        assert!(idle::get(&path, "thread").unwrap().is_some(), "{state}");
        assert_eq!(delivery::list_pending(&path).unwrap().len(), 1);
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite");
    let job = running(&path, 1);
    reserve::stage_usage_failure(&path, "thread", "typed usage").unwrap();
    delivery::stage_owned_queue_completion_with_release(
        &path,
        &job,
        "Failed",
        2.0,
        Some(("resident", 1)),
    )
    .unwrap();
    assert!(reserve::usage_failure_unresolved(&path, "thread").unwrap());
    assert!(idle::get(&path, "thread").unwrap().is_some());
}

#[test]
fn same_numeric_generation_and_old_journal_do_not_imply_current_resident() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite");
    running(&path, 1);
    assert!(
        !observed_completion::has_resident_evidence(&path, "thread", "turn", 1, "new").unwrap()
    );
    observed_completion::record_for_resident(&path, "thread", "turn", 1, "{}", "old").unwrap();
    assert!(observed_completion::has_resident_evidence(&path, "thread", "turn", 1, "old").unwrap());
    assert!(
        !observed_completion::has_resident_evidence(&path, "thread", "turn", 1, "new").unwrap()
    );
    observed_completion::record_for_resident(&path, "thread", "turn", 1, "{}", "new").unwrap();
    assert!(
        !observed_completion::has_resident_evidence(&path, "thread", "turn", 1, "new").unwrap()
    );
}

#[test]
fn legacy_usage_fence_does_not_invalidate_an_existing_idle_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite");
    let job = running(&path, 1);
    delivery::stage_owned_queue_completion_with_release(
        &path,
        &job,
        "Final",
        2.0,
        Some(("resident", 1)),
    )
    .unwrap();
    let intent = idle::get(&path, "thread").unwrap().unwrap();
    idle::verify(&path, &intent, true).unwrap();
    reserve::stage_usage_failure(&path, "thread", "late fence").unwrap();
    idle::verify(&path, &intent, true).unwrap();
    assert_eq!(idle::get(&path, "thread").unwrap().unwrap(), intent);
}
