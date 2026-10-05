use cdr_store::{
    ingress::{
        self, IngressKind, NewIngress, StoredIngress,
        stop::{StopScope, accept_nonrunning},
    },
    queue::{self, NewQueueJob},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;

fn enqueue(db: &Path, id: &str, now: f64) {
    queue::enqueue(
        db,
        NewQueueJob {
            job_id: id,
            target_thread_id: "target",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: None,
            app_server_generation: 7,
            prompt: "unchanged original input",
            queued: true,
            ack_sent: true,
            created_at: now,
        },
    )
    .unwrap();
}

fn scope() -> StopScope<'static> {
    StopScope {
        target: "target",
        channel: 99,
        owner: 20,
    }
}

fn admit_stop(db: &Path) -> (Value, StoredIngress) {
    let binding =
        json!({"target":"target","route":"Explicit","command":{"Stop":{"reference":"target"}}});
    ingress::admit(db, &NewIngress {
        ingress_id:"message:101".into(),kind:IngressKind::Message,event_id:Some(101),
        application_id:None,channel_id:99,owner_user_id:20,source_message_id:Some(101),
        payload:json!({"version":1,"plan":{"Execute":binding["command"]},"lifecycle_binding":binding}),
        target_thread_id:Some("target".into()),canonical_owner:None,now:2.0,
    }).unwrap();
    assert!(
        ingress::begin_execution(db, "message:101", "processing", Some("target"), 3.0).unwrap()
    );
    (binding, ingress::get(db, "message:101").unwrap().unwrap())
}

#[test]
fn stop_scope_survives_normal_result_confirmation_and_later_requests() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, "original", 1.0);
    Connection::open(&db).unwrap().execute(
        "INSERT INTO cdr_execution_holds VALUES ('original','target','previous uncertainty','preserved evidence',1)",
        [],
    ).unwrap();
    let original = serde_json::to_value(queue::list_filtered(&db, None, None).unwrap()).unwrap();
    let (binding, record) = admit_stop(&db);
    let receipt = accept_nonrunning(&db, scope(), &binding, Some(&record), || Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!(receipt.jobs, ["original"]);
    enqueue(&db, "later", 4.0);

    // This is the same result/confirmation sequence used after execute_plan.
    ingress::record_result(
        &db,
        "message:101",
        &json!({
            "response":"Stop accepted; original queued requests held: 1",
            "waits_for_final":false
        }),
        5.0,
    )
    .unwrap();
    ingress::confirm(&db, "message:101", 6.0).unwrap();
    let saved = ingress::get(&db, "message:101").unwrap().unwrap();
    assert_eq!(saved.state, "completed");
    assert!(saved.confirmation_delivered);
    assert!(
        !ingress::begin_execution(&db, "message:101", "processing", Some("target"), 7.0).unwrap()
    );
    assert!(accept_nonrunning(&db, scope(), &binding, Some(&record), || Ok(())).is_err());
    assert!(
        cdr_store::execution_hold::reason(&db, "later")
            .unwrap()
            .is_none()
    );
    let previous: (String, String) = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT reason,evidence_json FROM cdr_execution_holds WHERE job_id='original'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        previous,
        ("previous uncertainty".into(), "preserved evidence".into())
    );
    let jobs = queue::list_filtered(&db, None, None).unwrap();
    let preserved = jobs.iter().find(|job| job.job_id == "original").unwrap();
    assert_eq!(serde_json::to_value(preserved).unwrap(), original[0]);
    let saved_receipt = &saved.outcome.as_ref().unwrap()["stop_receipt"];
    assert_eq!(
        saved_receipt["jobs"],
        json!(["original"]),
        "normal message result storage erased the exact admitted stop scope"
    );
    assert_eq!(saved_receipt["kind"], "stop_accepted");
    assert_eq!(saved_receipt["execution_end_confirmed"], false);
    assert_repeat_results_preserve_receipt(&db, saved_receipt);
}

fn assert_repeat_results_preserve_receipt(db: &Path, expected: &Value) {
    ingress::record_result(
        db,
        "message:101",
        &json!({
            "response":"later confirmation",
            "stop_receipt":{"kind":"stop_accepted","jobs":["later"],"execution_end_confirmed":true}
        }),
        8.0,
    )
    .unwrap();
    let before = ingress::get(db, "message:101").unwrap().unwrap();
    assert_eq!(before.outcome.as_ref().unwrap()["stop_receipt"], *expected);
    assert!(ingress::record_result(db, "message:101", &Value::Null, 9.0).is_err());
    assert_eq!(
        ingress::get(db, "message:101").unwrap().as_ref(),
        Some(&before)
    );
    assert!(
        cdr_store::execution_hold::reason(db, "later")
            .unwrap()
            .is_none()
    );
}
