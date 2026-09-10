use cdr_store::{
    commentary_outbox, delivery, first_reply, goal_progress, ingress, mapping, prompt_intake,
    queue, room_cleanup,
};
use std::path::Path;

fn running(db: &Path) {
    mapping::upsert_thread(db, "thread", "project", "title", 100, 42, 1.0).unwrap();
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(801),
            app_server_generation: 1,
            prompt: "input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, "job", &[], 1).unwrap();
    queue::mark_running(db, "job", "turn", 1).unwrap();
}

#[test]
fn first_reply_uses_durable_ingress_confirmation_not_optimistic_queue_ack() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    ingress::admit(
        &db,
        &ingress::NewIngress {
            ingress_id: "message:801".into(),
            kind: ingress::IngressKind::Message,
            event_id: Some(801),
            application_id: None,
            channel_id: 42,
            owner_user_id: 3,
            source_message_id: Some(801),
            payload: serde_json::json!({"content":"input"}),
            target_thread_id: Some("thread".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    prompt_intake::admit_prompt_intake(
        &db,
        prompt_intake::NewPromptIntake {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(801),
            raw_prompt: "input",
            auto_queue_when_busy: false,
            require_current_mirror: false,
            created_at: 2.0,
        },
    )
    .unwrap();
    running(&db);
    assert!(queue::list(&db).unwrap()[0].ack_sent);
    assert_eq!(
        first_reply::pending(&db, "job").unwrap().as_deref(),
        Some("message:801")
    );
    ingress::record_result(
        &db,
        "message:801",
        &serde_json::json!({"response":"echo"}),
        3.0,
    )
    .unwrap();
    assert!(first_reply::pending(&db, "job").unwrap().is_some());
    ingress::confirm(&db, "message:801", 4.0).unwrap();
    assert!(first_reply::pending(&db, "job").unwrap().is_none());
}

#[test]
fn progress_remains_ordered_and_protects_deletion_after_queue_and_final_are_gone() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    running(&db);
    assert!(
        first_reply::pending(&db, "job").unwrap().is_none(),
        "headless fixture has no first reply"
    );
    let first = commentary_outbox::stage(&db, "thread", "turn", "one")
        .unwrap()
        .unwrap();
    let duplicate = commentary_outbox::stage(&db, "thread", "turn", "one")
        .unwrap()
        .unwrap();
    let second = commentary_outbox::stage(&db, "thread", "turn", "two")
        .unwrap()
        .unwrap();
    assert_eq!(first.sequence, duplicate.sequence);
    assert!(first.sequence < second.sequence);
    assert!(!commentary_outbox::has_pending(&db, "job", Some(first.sequence)).unwrap());
    assert!(commentary_outbox::has_pending(&db, "job", Some(second.sequence)).unwrap());
    let final_reply = delivery::stage_queue_completion(&db, "job", "Final", 5.0).unwrap();
    delivery::complete(&db, &final_reply.delivery_id).unwrap();
    assert_eq!(commentary_outbox::pending(&db).unwrap().len(), 2);
    assert!(
        room_cleanup::begin(&db, 42, Some("thread"), 6.0)
            .unwrap_err()
            .to_string()
            .contains("undelivered progress")
    );
    assert!(
        room_cleanup::archive::begin(&db, "thread", 6.0)
            .unwrap_err()
            .to_string()
            .contains("undelivered progress")
    );
    commentary_outbox::complete(&db, first.sequence).unwrap();
    assert!(!commentary_outbox::has_pending(&db, "job", Some(second.sequence)).unwrap());
    commentary_outbox::complete(&db, second.sequence).unwrap();
    room_cleanup::begin(&db, 42, Some("thread"), 7.0).unwrap();
    let connection = cdr_store::schema::open_initialized(&db).unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO codex_commentary_outbox
        (delivery_key,job_id,target_thread_id,turn_id,channel_id,text)
        VALUES ('late','job','thread','turn',42,'late')",
                []
            )
            .is_err()
    );
}

#[test]
fn legacy_goal_progress_is_preserved_without_guessing_a_request_owner() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let connection = cdr_store::schema::open_initialized(&db).unwrap();
    // Recreate an old-version table in this disposable migration fixture only.
    connection.execute_batch("DROP TABLE codex_goal_progress;
        CREATE TABLE codex_goal_progress(thread TEXT,turn TEXT,channel INTEGER,content TEXT,last_error TEXT,
            PRIMARY KEY(thread,turn));
        INSERT INTO codex_goal_progress VALUES ('thread','turn',42,'original progress','');").unwrap();
    drop(connection);
    let rows = goal_progress::pending(&db).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].content, "original progress");
    assert!(rows[0].job_id.is_none());
    mapping::upsert_thread(&db, "thread", "project", "title", 100, 42, 1.0).unwrap();
    assert!(
        room_cleanup::begin(&db, 42, Some("thread"), 2.0)
            .unwrap_err()
            .to_string()
            .contains("undelivered goal progress")
    );
    assert!(
        room_cleanup::archive::begin(&db, "thread", 2.0)
            .unwrap_err()
            .to_string()
            .contains("undelivered goal progress")
    );
}
