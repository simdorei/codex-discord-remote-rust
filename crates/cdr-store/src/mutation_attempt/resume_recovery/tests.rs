use super::*;
use crate::mutation_attempt::{NewAttempt, activate, begin, check};
use crate::queue::{NewQueueJob, enqueue, list, record_preflight_failure};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Candidate) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    activate(&db, "old-runtime").unwrap();
    begin(
        &db,
        &NewAttempt {
            runtime_id: "old-runtime",
            owner_id: "old-resident",
            generation: 1,
            attempt_id: "resume-a",
            wire_id: "wire-a",
            method: "thread/resume",
            target_thread_id: Some("A"),
            scoped: true,
            payload: &Value::Null,
        },
    )
    .unwrap();
    activate(&db, "new-runtime").unwrap();
    let candidate = candidates(&db, "new-runtime").unwrap().remove(0);
    (temp, db, candidate)
}

fn metadata() -> Value {
    json!({"thread":{"id":"A","status":{"type":"notLoaded"}}})
}
fn latest() -> Value {
    json!({"data":[{"id":"last-turn","status":"completed"}],"nextCursor":"older"})
}
fn recover_idle(db: &Path, candidate: &Candidate) -> Result<usize> {
    recover(
        db,
        "new-runtime",
        ("new-resident", 1),
        candidate,
        &metadata(),
        &latest(),
    )
}

#[test]
fn restart_releases_resume_and_only_unstarted_preflights_while_stop_does_not_block_new_work() {
    let (_temp, db, candidate) = fixture();
    let error = format!(
        "{}app-server mutation thread/resume outcome remains unknown: timeout; durable attempt resume-a retained; no automatic replay",
        execution_hold::PREFIX
    );
    for job in ["pending", "stopped", "unrelated"] {
        enqueue(
            &db,
            NewQueueJob {
                job_id: job,
                target_thread_id: "A",
                channel_id: 1,
                owner_user_id: None,
                discord_message_id: None,
                app_server_generation: 1,
                prompt: "fixture",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        record_preflight_failure(
            &db,
            job,
            1,
            if job == "unrelated" {
                "other error"
            } else {
                &error
            },
        )
        .unwrap();
    }
    let connection = existing(&db).unwrap();
    execution_hold::hold_in(&connection, "stopped", "A", "user stop", "{}").unwrap();
    assert!(check(&db, "new-runtime", Some("A")).is_err());
    assert_eq!(recover_idle(&db, &candidate).unwrap(), 1);
    check(&db, "new-runtime", Some("A")).unwrap();
    let jobs = list(&db).unwrap();
    assert!(
        jobs.iter()
            .find(|j| j.job_id == "pending")
            .unwrap()
            .last_error
            .is_empty()
    );
    assert_eq!(
        jobs.iter()
            .find(|j| j.job_id == "stopped")
            .unwrap()
            .last_error,
        error
    );
    assert_eq!(
        jobs.iter()
            .find(|j| j.job_id == "unrelated")
            .unwrap()
            .last_error,
        "other error"
    );
    assert!(
        !execution_hold::eligible_jobs(&db, jobs)
            .unwrap()
            .iter()
            .any(|j| j.job_id == "stopped")
    );
    let original: String = connection
        .query_row(
            "SELECT original_json FROM cdr_resume_restart_recoveries",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&original).unwrap()["state"],
        "prepared"
    );
    assert!(recover_idle(&db, &candidate).is_err());
    begin(
        &db,
        &NewAttempt {
            runtime_id: "new-runtime",
            owner_id: "new-resident",
            generation: 1,
            attempt_id: "new-request",
            wire_id: "new-wire",
            method: "thread/resume",
            target_thread_id: Some("A"),
            scoped: true,
            payload: &Value::Null,
        },
    )
    .unwrap();
}

#[test]
fn restart_requires_known_idle_exact_current_owner_and_unchanged_attempt() {
    let (_temp, db, candidate) = fixture();
    for value in [
        json!({}),
        json!({"thread":{"id":"wrong","status":{"type":"idle"}}}),
        json!({"thread":{"id":"A","status":{"type":"active"}}}),
    ] {
        assert!(
            recover(
                &db,
                "new-runtime",
                ("new-resident", 1),
                &candidate,
                &value,
                &latest()
            )
            .is_err()
        );
    }
    for value in [
        json!({}),
        json!({"data":[]}),
        json!({"data":[],"nextCursor":"more"}),
        json!({"data":[{"id":"x","status":"inProgress"}]}),
        json!({"data":[{"id":"x","status":"completed"}],"truncated":true}),
    ] {
        assert!(
            recover(
                &db,
                "new-runtime",
                ("new-resident", 1),
                &candidate,
                &metadata(),
                &value
            )
            .is_err()
        );
    }
    assert!(
        recover(
            &db,
            "new-runtime",
            ("old-resident", 1),
            &candidate,
            &metadata(),
            &latest()
        )
        .is_err()
    );
    assert!(
        recover(
            &db,
            "old-runtime",
            ("new-resident", 1),
            &candidate,
            &metadata(),
            &latest()
        )
        .is_err()
    );
    existing(&db)
        .unwrap()
        .execute(
            "UPDATE codex_mutation_attempts SET request_sha256='changed'",
            [],
        )
        .unwrap();
    assert!(recover_idle(&db, &candidate).is_err());
    assert!(check(&db, "new-runtime", Some("A")).is_err());
}

#[test]
fn restart_never_retires_turn_start_settings_or_current_runtime_resume() {
    for method in [
        "turn/start",
        "thread/settings/update",
        "thread/start",
        "thread/resume",
    ] {
        let (_temp, db, _) = fixture();
        let c = existing(&db).unwrap();
        c.execute(
            "UPDATE codex_mutation_attempts SET method=?,runtime_id=?",
            params![
                method,
                if method == "thread/resume" {
                    "new-runtime"
                } else {
                    "old-runtime"
                }
            ],
        )
        .unwrap();
        assert!(candidates(&db, "new-runtime").unwrap().is_empty());
        assert!(check(&db, "new-runtime", Some("A")).is_err());
    }
}

#[test]
fn restart_audit_failure_rolls_back_and_active_queue_still_blocks() {
    let (_temp, db, candidate) = fixture();
    let c = existing(&db).unwrap();
    c.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON cdr_resume_restart_recoveries BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(recover_idle(&db, &candidate).is_err());
    assert!(check(&db, "new-runtime", Some("A")).is_err());
    c.execute_batch("DROP TRIGGER fail").unwrap();
    enqueue(
        &db,
        NewQueueJob {
            job_id: "active",
            target_thread_id: "A",
            channel_id: 1,
            owner_user_id: None,
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "fixture",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    c.execute("UPDATE codex_turn_queue SET state='starting'", [])
        .unwrap();
    assert!(recover_idle(&db, &candidate).is_err());
    let count: i64 = c
        .query_row(
            "SELECT count(*) FROM cdr_resume_restart_recoveries",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
