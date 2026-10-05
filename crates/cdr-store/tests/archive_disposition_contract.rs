#[path = "support/recovery_abandonment_fixture.rs"]
mod fixture;

use cdr_store::{
    async_resolution::abandonment as a,
    ingress::{self, IngressKind, NewIngress},
};
use fixture::{Fixture, JOB, TARGET};
use rusqlite::params;
use serde_json::{Value, json};

const LEGACY_VIEW: &str = "CREATE VIEW IF NOT EXISTS cdr_archive_inspections_v1 AS
SELECT ingress_id FROM discord_ingress_journal WHERE
    (kind='message' AND json_extract(payload_json,'$.version')=1 AND (
        json_extract(payload_json,'$.plan.Execute') IN ('Help','Runners','Doctor','Where','Identity','Resources')
        OR json_type(payload_json,'$.plan.Execute.SavedRequest')='object'))
    OR (kind='interaction' AND json_extract(payload_json,'$.version')=1
        AND json_extract(payload_json,'$.work.Slash.name') IN ('help','runners','doctor','where'));";

fn fence(f: &Fixture, phase: &str) {
    f.db.execute(
        "INSERT INTO codex_archive_fences(target_thread_id,operation_id,phase) VALUES(?,'archive-proof',?)",
        params![TARGET, phase],
    ).unwrap();
}

fn command() -> Value {
    json!({
        "version":1,"processing_mode":"normal","author_is_bot":false,
        "content":format!("!discard-request {JOB}"),
        "plan":{"Execute":{"DiscardRequest":{"job_id":JOB}}},
        "settings_binding":null,"lifecycle_binding":null
    })
}

fn admit(f: &Fixture, payload: Value, kind: IngressKind) -> String {
    let interaction = matches!(kind, IngressKind::Interaction);
    let id = if interaction {
        "interaction:91"
    } else {
        "message:81"
    };
    ingress::admit(
        &f.path,
        &NewIngress {
            ingress_id: id.into(),
            kind,
            event_id: Some(if interaction { 91 } else { 81 }),
            application_id: if interaction { Some(50) } else { None },
            channel_id: 20,
            owner_user_id: 30,
            source_message_id: Some(if interaction { 60 } else { 81 }),
            payload,
            target_thread_id: Some(TARGET.into()),
            canonical_owner: None,
            now: 3.0,
        },
    )
    .unwrap();
    id.into()
}

fn assert_fenced(f: &Fixture, phase: &str) {
    let actual: (String, String) =
        f.db.query_row(
            "SELECT operation_id,phase FROM codex_archive_fences WHERE target_thread_id=?",
            [TARGET],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(actual, ("archive-proof".into(), phase.into()));
    assert!(cdr_store::async_resolution::admission_held(&f.path, TARGET).unwrap());
}

#[test]
fn attempted_and_verified_fences_allow_only_local_discard_command_custody() {
    for phase in ["attempted", "verified"] {
        let f = Fixture::new();
        assert_eq!(f.path.parent(), Some(f.temp.path()));
        fence(&f, phase);
        let id = admit(&f, command(), IngressKind::Message);
        assert!(ingress::begin_execution(&f.path, &id, "processing", Some(TARGET), 4.0).unwrap());
        assert_eq!(f.count("codex_turn_queue"), 2);
        assert_eq!(f.count("codex_request_cancellations"), 0);
        assert_fenced(&f, phase);
    }
}

#[test]
fn archived_exact_decision_disposes_one_request_but_retains_fence_and_thread_hold() {
    for phase in ["attempted", "verified"] {
        let f = Fixture::new();
        fence(&f, phase);
        f.delivered();
        f.click(a::Decision::AbandonOnly);
        assert!(f.apply(a::Decision::AbandonOnly).is_ok());
        assert_eq!(f.count("codex_turn_queue"), 1);
        assert_eq!(f.count("codex_request_cancellations"), 1);
        assert_fenced(&f, phase);
        let before = queue_snapshot(&f, fixture::SIBLING);
        let start_error =
            f.db.execute(
                "UPDATE codex_turn_queue SET state='starting' WHERE job_id=?",
                [fixture::SIBLING],
            )
            .unwrap_err();
        eprintln!("incident start remains blocked: {start_error}");
        assert_eq!(queue_snapshot(&f, fixture::SIBLING), before);
        let insert_error =
            f.db.execute(
                "INSERT INTO codex_turn_queue SELECT * FROM codex_turn_queue WHERE job_id=?",
                [fixture::SIBLING],
            )
            .unwrap_err();
        eprintln!("incident queue handoff remains blocked: {insert_error}");
        assert_eq!(queue_snapshot(&f, fixture::SIBLING), before);
        assert_eq!(f.count("codex_turn_queue"), 1);
        assert_eq!(f.count("codex_request_cancellations"), 1);
        assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 1);
        assert_fenced(&f, phase);
    }
}

#[test]
fn unknown_mixed_bot_and_mutating_message_shapes_stay_archive_held() {
    let mut cases = vec![
        json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"never replay"}}}}),
        json!({"version":1,"plan":{"Execute":{"Settings":{"model":"reserve"}}}}),
    ];
    for (key, value) in [
        ("author_is_bot", json!(true)),
        ("processing_mode", json!("sealed")),
    ] {
        let mut p = command();
        p[key] = value;
        cases.push(p);
    }
    let mut p = command();
    p["plan"]["Execute"]["Ask"] = json!({"prompt":"mixed"});
    cases.push(p);
    let mut p = command();
    p["plan"]["Execute"]["DiscardRequest"]["extra"] = json!(1);
    cases.push(p);
    let mut p = command();
    p["plan"]["Execute"]["DiscardRequest"]["job_id"] = json!(fixture::ID);
    cases.push(p);
    let mut p = command();
    p["work"] = json!({"Component":{"Approval":{}}});
    cases.push(p);
    let mut p = command();
    p["settings_binding"] = json!({"target":TARGET});
    cases.push(p);
    for payload in cases {
        let f = Fixture::new();
        fence(&f, "attempted");
        let id = admit(&f, payload, IngressKind::Message);
        assert!(!matches!(
            ingress::begin_execution(&f.path, &id, "processing", Some(TARGET), 4.0),
            Ok(true)
        ));
        let row = ingress::get(&f.path, &id).unwrap().unwrap();
        assert_eq!(row.phase, "archive_fenced");
        f.unchanged();
        assert_fenced(&f, "attempted");
    }
}

#[test]
fn unrelated_or_malformed_components_cannot_borrow_local_discard_exemption() {
    for component in [
        json!({"RecoveryPublicationDecision":{"proposal_id":fixture::ID,"revision":1,"decision":"ApproveExact"}}),
        json!({"RecoveryAbandonDecision":{"proposal_id":fixture::ID,"revision":1,"decision":"ApproveExact"}}),
        json!({"RecoveryAbandonDecision":{"proposal_id":fixture::ID,"revision":0,"decision":"AbandonOnly"}}),
        json!({"RecoveryAbandonDecision":{"proposal_id":"bad","revision":1,"decision":"AbandonOnly"}}),
        json!({"RecoveryAbandonDecision":{"proposal_id":fixture::ID,"revision":1,"decision":"AbandonOnly","extra":true}}),
        json!({"Approval":{"thread_id":TARGET,"answer":"Approve"}}),
    ] {
        let f = Fixture::new();
        fence(&f, "verified");
        let id = admit(
            &f,
            json!({"version":1,"work":{"Component":component}}),
            IngressKind::Interaction,
        );
        assert_eq!(
            ingress::get(&f.path, &id).unwrap().unwrap().phase,
            "archive_fenced"
        );
        f.unchanged();
        assert_fenced(&f, "verified");
    }
}

#[test]
fn known_legacy_view_upgrades_without_removing_fences_or_replaying_saved_work() {
    let f = Fixture::new();
    fence(&f, "attempted");
    let old = admit(
        &f,
        json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"old"}}}}),
        IngressKind::Message,
    );
    f.db.execute_batch("DROP VIEW cdr_archive_inspections_v1;")
        .unwrap();
    f.db.execute_batch(LEGACY_VIEW).unwrap();
    let db = cdr_store::schema::open_initialized(&f.path).unwrap();
    let definition: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='cdr_archive_inspections_v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(definition.contains("DiscardRequest"));
    assert!(definition.contains("RecoveryAbandonDecision"));
    assert_eq!(
        ingress::get(&f.path, &old).unwrap().unwrap().phase,
        "archive_fenced"
    );
    assert!(!matches!(
        ingress::begin_execution(&f.path, &old, "processing", Some(TARGET), 4.0),
        Ok(true)
    ));
    f.unchanged();
    assert_fenced(&f, "attempted");
}

#[test]
fn unknown_view_definition_is_not_silently_accepted_or_replaced() {
    let f = Fixture::new();
    fence(&f, "attempted");
    f.db.execute_batch("DROP VIEW cdr_archive_inspections_v1;")
        .unwrap();
    f.db.execute_batch(&LEGACY_VIEW.replace("'Resources'", "'resources'"))
        .unwrap();
    let before: String =
        f.db.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='cdr_archive_inspections_v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(cdr_store::schema::open_initialized(&f.path).is_err());
    let after: String =
        f.db.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='cdr_archive_inspections_v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(f.count("codex_turn_queue"), 2);
    assert_eq!(f.count("codex_request_cancellations"), 0);
    // Read the unchanged fence directly: initialized APIs correctly reject this schema.
    let phase: String =
        f.db.query_row(
            "SELECT phase FROM codex_archive_fences WHERE target_thread_id=?",
            [TARGET],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(phase, "attempted");
}

// Preserve every SQLite value, including storage class and REAL bit pattern.
fn queue_snapshot(f: &Fixture, job: &str) -> Value {
    use rusqlite::types::ValueRef;

    let mut statement =
        f.db.prepare("SELECT * FROM codex_turn_queue WHERE job_id=?")
            .unwrap();
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|s| (*s).into())
        .collect();
    statement
        .query_row([job], |row| {
            let mut values = Vec::new();
            for index in 0..columns.len() {
                values.push(match row.get_ref(index)? {
                    ValueRef::Null => json!(["null"]),
                    ValueRef::Integer(value) => json!(["integer", value]),
                    ValueRef::Real(value) => json!(["real_bits", value.to_bits().to_string()]),
                    ValueRef::Text(value) => json!(["text", std::str::from_utf8(value).unwrap()]),
                    ValueRef::Blob(value) => json!(["blob", value]),
                });
            }
            Ok(json!({"columns":columns,"values":values}))
        })
        .unwrap()
}

const ORDINARY: &str = "archive-only-control";
const ORDINARY_JOB: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const ORDINARY_NEXT: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";

fn enqueue_ordinary(f: &Fixture, job: &str, event: i64) -> cdr_store::Result<()> {
    cdr_store::queue::enqueue(
        &f.path,
        cdr_store::queue::NewQueueJob {
            job_id: job,
            target_thread_id: ORDINARY,
            channel_id: 21,
            owner_user_id: Some(30),
            discord_message_id: Some(event),
            app_server_generation: 1,
            prompt: "private archive control",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )?;
    Ok(())
}

#[test]
fn independent_archive_fence_blocks_otherwise_executable_queue_without_rewinding_starting() {
    // The unfenced positive control is a separate database; no Starting row is
    // reset to Pending and no existing hold or trigger is removed for this test.
    let positive = Fixture::new();
    enqueue_ordinary(&positive, ORDINARY_JOB, 72).unwrap();
    assert!(!cdr_store::async_resolution::admission_held(&positive.path, ORDINARY).unwrap());
    assert_eq!(
        positive
            .db
            .execute(
                "UPDATE codex_turn_queue SET state='starting' WHERE job_id=?",
                [ORDINARY_JOB],
            )
            .unwrap(),
        1
    );
    enqueue_ordinary(&positive, ORDINARY_NEXT, 73).unwrap();

    for phase in ["attempted", "verified"] {
        let f = Fixture::new();
        enqueue_ordinary(&f, ORDINARY_JOB, 72).unwrap();
        assert!(!cdr_store::async_resolution::admission_held(&f.path, ORDINARY).unwrap());
        let before = queue_snapshot(&f, ORDINARY_JOB);
        let original = queue_snapshot(&f, JOB);
        let sibling = queue_snapshot(&f, fixture::SIBLING);
        f.db.execute(
            "INSERT INTO codex_archive_fences(target_thread_id,operation_id,phase)
             VALUES(?,'archive-only-proof',?)",
            params![ORDINARY, phase],
        )
        .unwrap();
        let error =
            f.db.execute(
                "UPDATE codex_turn_queue SET state='starting' WHERE job_id=?",
                [ORDINARY_JOB],
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("archive fence prevents queue execution"),
            "{error}"
        );
        let error = enqueue_ordinary(&f, ORDINARY_NEXT, 73).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("archive fence prevents queue handoff"),
            "{error}"
        );
        assert_eq!(queue_snapshot(&f, ORDINARY_JOB), before);
        assert_eq!(queue_snapshot(&f, JOB), original);
        assert_eq!(queue_snapshot(&f, fixture::SIBLING), sibling);
        assert_eq!(f.count("codex_turn_queue"), 3);
        assert_eq!(f.count("codex_request_cancellations"), 0);
        let actual: (String,String,Option<String>) = f.db.query_row(
            "SELECT operation_id,phase,own_ingress_id FROM codex_archive_fences WHERE target_thread_id=?",
            [ORDINARY], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        assert_eq!(actual, ("archive-only-proof".into(), phase.into(), None));
    }
}
