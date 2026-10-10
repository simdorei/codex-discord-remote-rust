use cdr_store::{
    async_resolution as resolution, delivery, execution_hold, ingress, queue,
    schema::open_initialized,
};
use rusqlite::params;
use serde_json::json;
use std::path::Path;

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

const STOP: &str = "message:39";
const ORIGIN: &str = "message:40";
const TERMINAL: &str = r#"{"threadId":"thread-b","turn":{"id":"original","status":"completed"}}"#;

fn legacy_stop(path: &Path, command: &str) {
    let db = open_initialized(path).unwrap();
    db.execute(
        "INSERT INTO discord_ingress_journal
         (ingress_id,kind,event_id,source_message_id,channel_id,owner_user_id,
          payload_json,state,phase,target_thread_id,created_at,updated_at)
         VALUES(?,'message',39,39,20,30,?,'held','processing','thread-b',1000,1000)",
        params![
            STOP,
            json!({"version":1,"plan":{"Execute":{command:{"reference":null}}},
            "lifecycle_binding":null})
            .to_string()
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO cdr_recovery_ingress_order(ingress_id,kind,event_id,origin)
         VALUES(?,'message',39,'legacy')",
        [STOP],
    )
    .unwrap();
}

fn origin(path: &Path) {
    ingress::admit(
        path,
        &ingress::NewIngress {
            ingress_id: ORIGIN.into(),
            kind: ingress::IngressKind::Message,
            event_id: Some(40),
            application_id: None,
            channel_id: 20,
            owner_user_id: 30,
            source_message_id: Some(40),
            target_thread_id: Some("thread-b".into()),
            canonical_owner: None,
            now: 1.0,
            payload: json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"original input"}}}}),
        },
    )
    .unwrap();
}

fn execution(path: &Path, complete: bool) {
    fixture::dispatching(path, "resident");
    open_initialized(path)
        .unwrap()
        .execute(
            "UPDATE discord_ingress_journal SET owner_kind='prompt',owner_id='origin',
         state='owned',phase='result_recorded' WHERE ingress_id=?",
            [ORIGIN],
        )
        .unwrap();
    if complete {
        resolution::record_terminal_notification(
            path, "thread-b", "original", 1, "resident", TERMINAL,
        )
        .unwrap();
        let owner = queue::list(path).unwrap().remove(0);
        delivery::stage_owned_queue_completion_with_release(
            path,
            &owner,
            "Final",
            4.0,
            Some(("resident", 1)),
        )
        .unwrap();
    }
}

#[test]
fn old_unbound_stop_does_not_reappear_after_a_newer_request_finishes_its_async_question() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    legacy_stop(&path, "Stop");
    let original_stop = ingress::get(&path, STOP).unwrap().unwrap();
    assert!(!resolution::admission_held(&path, "thread-b").unwrap());
    origin(&path);
    execution(&path, true);
    assert!(
        !resolution::admission_held(&path, "thread-b").unwrap(),
        "the old stop became a new thread-wide hold after an unrelated question completed"
    );
    assert_eq!(ingress::get(&path, STOP).unwrap(), Some(original_stop));
    fixture::pending(&path, "next", "thread-b", 1);
    assert!(
        queue::try_begin_attempt(&path, "next", &[], 1)
            .unwrap()
            .is_some()
    );
}

#[test]
fn later_stop_archive_and_unproven_completion_still_hold_admission() {
    for case in [
        "later_stop",
        "archive",
        "active",
        "no_origin",
        "other_owner",
        "other_channel",
        "other_job",
        "other_input_event",
        "bound_stop",
        "missing_certificate",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite");
        if case == "later_stop" {
            origin(&path);
        }
        legacy_stop(&path, if case == "archive" { "Archive" } else { "Stop" });
        if case != "later_stop" {
            origin(&path);
        }
        execution(&path, case != "active");
        let db = open_initialized(&path).unwrap();
        match case {
            "no_origin" => {
                db.execute(
                    "UPDATE discord_ingress_journal SET owner_id=NULL WHERE ingress_id=?",
                    [ORIGIN],
                )
                .unwrap();
            }
            "other_owner" => {
                db.execute(
                    "UPDATE discord_ingress_journal SET owner_user_id=31 WHERE ingress_id=?",
                    [ORIGIN],
                )
                .unwrap();
            }
            "other_channel" => {
                db.execute(
                    "UPDATE discord_ingress_journal SET channel_id=21 WHERE ingress_id=?",
                    [ORIGIN],
                )
                .unwrap();
            }
            "other_job" => {
                db.execute("UPDATE discord_ingress_journal SET owner_id='different-job' WHERE ingress_id=?", [ORIGIN]).unwrap();
            }
            "other_input_event" => {
                db.execute("UPDATE discord_ingress_journal SET event_id=41,source_message_id=41 WHERE ingress_id=?", [ORIGIN]).unwrap();
            }
            "bound_stop" => {
                db.execute("UPDATE discord_ingress_journal SET payload_json=json_set(payload_json,'$.lifecycle_binding',json(?)) WHERE ingress_id=?", params![json!({"target":"thread-b","route":"Mapped","command":{"Stop":{"reference":null}}}).to_string(), STOP]).unwrap();
            }
            "missing_certificate" => {
                db.execute(
                    "UPDATE cdr_async_execution_obligations SET revision=revision+1",
                    [],
                )
                .unwrap();
            }
            _ => {}
        }
        let before = ingress::get(&path, STOP).unwrap();
        assert!(
            resolution::admission_held(&path, "thread-b").unwrap(),
            "{case}"
        );
        assert_eq!(ingress::get(&path, STOP).unwrap(), before, "{case}");
        assert!(
            !resolution::admission_held(&path, "unrelated").unwrap(),
            "{case}"
        );
    }
}

#[test]
fn missing_or_legacy_input_order_is_not_proof_of_a_new_request() {
    for legacy_order in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite");
        legacy_stop(&path, "Stop");
        let db = open_initialized(&path).unwrap();
        db.execute(
            "INSERT INTO discord_ingress_journal
             (ingress_id,kind,event_id,source_message_id,channel_id,owner_user_id,
              payload_json,state,phase,target_thread_id,created_at,updated_at)
             VALUES(?,'message',40,40,20,30,'{}','owned','result_recorded','thread-b',1,1)",
            [ORIGIN],
        )
        .unwrap();
        if legacy_order {
            db.execute(
                "INSERT INTO cdr_recovery_ingress_order(ingress_id,kind,event_id,origin)
                 VALUES(?,'message',40,'legacy')",
                [ORIGIN],
            )
            .unwrap();
        }
        execution(&path, true);
        assert!(resolution::admission_held(&path, "thread-b").unwrap());
    }
}

#[test]
fn superseded_legacy_stop_does_not_release_requests_the_user_already_stopped() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    legacy_stop(&path, "Stop");
    origin(&path);
    execution(&path, true);
    fixture::pending(&path, "stopped", "thread-b", 1);
    let db = open_initialized(&path).unwrap();
    db.execute("INSERT INTO cdr_execution_holds VALUES('stopped','thread-b','user stopped this request','{}',5)", []).unwrap();
    let before = queue::list(&path).unwrap();
    assert!(!resolution::admission_held(&path, "thread-b").unwrap());
    assert!(
        queue::try_begin_attempt(&path, "stopped", &[], 1)
            .unwrap()
            .is_none()
    );
    assert_eq!(queue::list(&path).unwrap(), before);
    assert!(execution_hold::reason(&path, "stopped").unwrap().is_some());
}
