use cdr_store::{
    archive_fence,
    ingress::{self, IngressKind, NewIngress},
    schema,
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Barrier},
};

fn request(target: Option<&str>) -> NewIngress {
    NewIngress {
        ingress_id: "message:70".into(),
        kind: IngressKind::Message,
        event_id: Some(70),
        application_id: None,
        channel_id: 99,
        owner_user_id: 20,
        source_message_id: Some(70),
        payload: json!({"version":1,"content":"  보존할 요청\n원문  ","plan":{"Execute":{"Ask":{"prompt":"saved"}}}}),
        target_thread_id: target.map(str::to_owned),
        canonical_owner: None,
        now: 3.0,
    }
}

fn scope() -> BTreeSet<String> {
    ["root".into(), "child".into()].into()
}

fn count(db: &Path) -> i64 {
    schema::open_initialized(db)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM codex_archive_fences", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn simultaneous_admission_and_reservation_have_exactly_one_executable_winner() {
    for index in 0..20 {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        drop(schema::open_initialized(&db).unwrap());
        let request = request(Some(if index % 2 == 0 { "root" } else { "child" }));
        let gate = Arc::new(Barrier::new(2));
        let archive = {
            let db = db.clone();
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                archive_fence::reserve(&db, &scope(), None)
            })
        };
        gate.wait();
        let record = ingress::admit(&db, &request).unwrap().record.unwrap();
        let reservation = archive.join().unwrap();
        assert_eq!(record.payload, request.payload);
        if reservation.is_ok() {
            assert_eq!(record.state, "held");
            assert_eq!(count(&db), 2);
            assert!(!ingress::begin_execution(&db, "message:70", "processing", None, 4.0).unwrap());
        } else {
            assert_eq!(record.state, "staged");
            assert_eq!(count(&db), 0);
            assert!(ingress::begin_execution(&db, "message:70", "processing", None, 4.0).unwrap());
        }
    }
}

#[test]
fn archive_fences_survive_reopen_and_do_not_allow_direct_handoff_or_retargeting() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let operation = archive_fence::reserve(&db, &scope(), None).unwrap();
    let request = request(Some("child"));
    ingress::admit(&db, &request).unwrap();
    ingress::recover_prior_runtime(&db, "new-runtime", 100_000.0).unwrap();
    assert_eq!(count(&db), 2);
    let queue = cdr_store::queue::NewQueueJob {
        job_id: "q",
        target_thread_id: "child",
        channel_id: 99,
        owner_user_id: Some(20),
        discord_message_id: Some(70),
        app_server_generation: 1,
        prompt: "saved",
        queued: true,
        ack_sent: false,
        created_at: 4.0,
    };
    assert!(
        cdr_store::queue::enqueue(&db, queue)
            .unwrap_err()
            .to_string()
            .contains("archive fence")
    );
    let intake = cdr_store::prompt_intake::NewPromptIntake {
        job_id: "i",
        target_thread_id: "child",
        channel_id: 99,
        owner_user_id: Some(20),
        discord_message_id: Some(70),
        raw_prompt: "saved",
        auto_queue_when_busy: true,
        require_current_mirror: false,
        created_at: 4.0,
    };
    assert!(
        cdr_store::prompt_intake::admit_prompt_intake(&db, intake)
            .unwrap_err()
            .to_string()
            .contains("archive fence")
    );
    let connection = schema::open_initialized(&db).unwrap();
    assert!(connection.execute("UPDATE discord_ingress_journal SET state='executing' WHERE ingress_id='message:70'", []).is_err());
    assert!(
        cdr_store::queue::enqueue(
            &db,
            cdr_store::queue::NewQueueJob {
                target_thread_id: "other",
                discord_message_id: None,
                ..queue
            }
        )
        .is_ok()
    );
    assert!(
        connection
            .execute(
                "UPDATE codex_turn_queue SET target_thread_id='root' WHERE job_id='q'",
                []
            )
            .is_err()
    );
    assert!(
        cdr_store::prompt_intake::admit_prompt_intake(
            &db,
            cdr_store::prompt_intake::NewPromptIntake {
                target_thread_id: "other",
                discord_message_id: None,
                ..intake
            }
        )
        .is_ok()
    );
    assert!(
        connection
            .execute(
                "UPDATE codex_prompt_intakes SET target_thread_id='root' WHERE job_id='i'",
                []
            )
            .is_err()
    );
    archive_fence::verified(&db, &operation).unwrap();
    ingress::hold(&db, "message:70", "generic processing failure", true, 5.0).unwrap();
    let held = ingress::get(&db, "message:70").unwrap().unwrap();
    assert_eq!(held.payload, request.payload);
    assert_eq!(held.state, "held");
    assert!(held.hold_reason.contains("archive scope"));
    assert_eq!(count(&db), 2);
}

#[test]
fn archive_inspection_remains_usable_but_unknown_work_is_held_until_outcome_known() {
    for (kind, plan) in [
        (
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":"Runners"}}),
        ),
        (
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":{"SavedRequest":{"request_id":"old"}}}}),
        ),
        (
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":"Help"}}),
        ),
        (
            IngressKind::Interaction,
            json!({"version":1,"work":{"Slash":{"name":"runners"}}}),
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        archive_fence::reserve(&db, &scope(), None).unwrap();
        let mut inspection = request(Some("root"));
        inspection.kind = kind;
        inspection.payload = plan;
        let row = ingress::admit(&db, &inspection).unwrap().record.unwrap();
        assert_eq!(row.state, "staged");
        assert!(
            ingress::begin_execution(&db, &row.ingress_id, "processing", Some("root"), 4.0)
                .unwrap()
        );
    }
    for verified in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        let operation = archive_fence::reserve(&db, &scope(), None).unwrap();
        if verified {
            archive_fence::verified(&db, &operation).unwrap();
        }
        let row = ingress::admit(&db, &request(None)).unwrap().record.unwrap();
        assert_eq!(row.state, if verified { "staged" } else { "held" });
        if verified {
            assert!(
                ingress::begin_execution(&db, &row.ingress_id, "processing", Some("root"), 4.0)
                    .unwrap_err()
                    .to_string()
                    .contains("archive fence")
            );
        }
    }
}

#[test]
fn missing_archive_extension_is_backed_up_and_repaired_without_changing_shared_v2() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let mut connection = schema::open_initialized(&db).unwrap();
    connection
        .execute_batch("DROP TRIGGER cdr_archive_queue_insert_v1;")
        .unwrap();
    let backup = schema::initialize(&mut connection, &db).unwrap().unwrap();
    assert!(backup.is_file());
    assert_eq!(schema::schema_version(&connection).unwrap(), 2);
    assert!(schema::initialize(&mut connection, &db).unwrap().is_none());
    let previous = rusqlite::Connection::open(backup).unwrap();
    assert_eq!(
        previous
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name='cdr_archive_queue_insert_v1'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
