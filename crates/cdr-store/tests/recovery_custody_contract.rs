use cdr_store::{ingress, mapping, schema};
use serde_json::json;

fn fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    ingress::StoredIngress,
) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mapping::upsert_thread(&db, "original", "project", "title", 100, 42, 1.0).unwrap();
    let command = json!({"Recover":{"reference":null}});
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
            payload: json!({"version":1,"plan":{"Execute":command},
            "lifecycle_binding":{"target":"original","route":"Mapped","command":command}}),
            target_thread_id: Some("original".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    assert!(
        ingress::begin_execution(&db, "message:801", "processing", Some("original"), 2.0).unwrap()
    );
    let record = ingress::get(&db, "message:801").unwrap().unwrap();
    (temp, db, record)
}

#[test]
fn recovery_claim_is_once_only_and_durable_across_a_fresh_connection() {
    let (_temp, db, original) = fixture();
    let claim = ingress::claim_recovery(&db, &original).unwrap();
    claim.validate(&db).unwrap();
    assert!(ingress::claim_recovery(&db, &original).is_err());
    let cold = ingress::get(&db, "message:801").unwrap().unwrap();
    assert_eq!(cold.phase, "recovery_claimed");
    assert!(ingress::claim_recovery(&db, &cold).is_err());
    assert!(
        !ingress::begin_execution(&db, "message:801", "processing", Some("original"), 3.0).unwrap()
    );
}

#[test]
fn stale_or_rewritten_admission_never_retains_recovery_authority() {
    for sql in [
        "UPDATE discord_ingress_journal SET owner_user_id=4",
        "UPDATE discord_ingress_journal SET channel_id=43",
        "UPDATE discord_ingress_journal SET event_id=802",
        "UPDATE discord_ingress_journal SET target_thread_id='replacement'",
        "UPDATE discord_ingress_journal SET runtime_id='new-runtime'",
        "UPDATE discord_ingress_journal SET state='held'",
        "UPDATE discord_ingress_journal SET payload_json=json_set(payload_json,'$.version',2)",
    ] {
        let (_temp, db, original) = fixture();
        let claim = ingress::claim_recovery(&db, &original).unwrap();
        schema::open_initialized(&db)
            .unwrap()
            .execute(sql, [])
            .unwrap();
        assert!(claim.validate(&db).is_err(), "{sql}");
    }
}

#[test]
fn remap_and_database_failure_are_not_permission_to_recover_a_replacement() {
    let (_temp, db, original) = fixture();
    let claim = ingress::claim_recovery(&db, &original).unwrap();
    mapping::upsert_thread(&db, "original", "project", "A", 100, 43, 3.0).unwrap();
    mapping::upsert_thread(&db, "replacement", "project", "B", 100, 42, 3.0).unwrap();
    assert!(claim.validate(&db).is_err());
    let disconnected = rusqlite::Connection::open_in_memory().unwrap();
    assert!(claim.validate_in(&disconnected).is_err());
}

#[test]
fn binding_syntax_cannot_turn_an_omitted_reference_into_an_explicit_bypass() {
    for change in [
        "json_remove(payload_json,'$.lifecycle_binding')",
        "json_set(payload_json,'$.lifecycle_binding.route','Explicit')",
        "json_set(payload_json,'$.lifecycle_binding.target','replacement')",
        "json_set(payload_json,'$.lifecycle_binding.command.Recover.reference','replacement')",
    ] {
        let (_temp, db, _) = fixture();
        schema::open_initialized(&db)
            .unwrap()
            .execute(
                &format!("UPDATE discord_ingress_journal SET payload_json={change}"),
                [],
            )
            .unwrap();
        let altered = ingress::get(&db, "message:801").unwrap().unwrap();
        assert!(ingress::claim_recovery(&db, &altered).is_err(), "{change}");
        assert_eq!(
            ingress::get(&db, "message:801").unwrap().unwrap().phase,
            "processing"
        );
    }
}
