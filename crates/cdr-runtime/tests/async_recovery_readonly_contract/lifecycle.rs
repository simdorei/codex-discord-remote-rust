use super::*;
use cdr_store::{
    ingress::stop::{StopScope, accept_unresolved},
    queue,
    schema::open_initialized,
};
use serde_json::json;

fn binding() -> Value {
    json!({"target":"thread-b","route":"Mapped","command":{"Stop":{"reference":null}}})
}

fn accept_stop(path: &Path) {
    let receipt = accept_unresolved(
        path,
        StopScope {
            target: "thread-b",
            channel: 20,
            owner: 30,
        },
        &binding(),
        None,
        || Ok(()),
    )
    .unwrap()
    .unwrap();
    assert!(!receipt.jobs.is_empty());
}

#[tokio::test]
async fn accepted_stop_and_later_archive_intent_are_visible_without_claiming_execution_end() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    fixture::dispatching(&path, "resident");
    queue::complete(&path, "origin").unwrap();
    fixture::pending(&path, "next", "thread-b", 2);
    accept_stop(&path);
    let db = open_initialized(&path).unwrap();
    for (id, target, action) in [
        ("archive-request", Some("thread-b"), "Archive"),
        ("unbound-stop", None, "Stop"),
        ("unrelated", Some("other-thread"), "Stop"),
    ] {
        let payload = json!({"plan":{"Execute":{(action):{"reference":null}}},"private":"do not leak this prompt"});
        db.execute(
            "INSERT INTO discord_ingress_journal
            (ingress_id,kind,channel_id,owner_user_id,payload_json,state,phase,
             target_thread_id,created_at,updated_at)
            VALUES(?1,'message',20,30,?2,'held','processing',?3,10,10)",
            rusqlite::params![id, payload.to_string(), target],
        )
        .unwrap();
    }
    drop(db);
    let before = file_sha(&path);
    let jobs = queue::list(&path).unwrap();
    let report = inspect(&path, &[]).await.unwrap();
    let controls = &report["lifecycle_evidence"];
    assert_eq!(controls["diagnostic_only"], true);
    assert_eq!(controls["does_not_confirm_execution_end"], true);
    assert_eq!(controls["automatic_resume_authorized"], false);
    assert_eq!(
        controls["stop_receipts"]["rows"].as_array().unwrap().len(),
        1
    );
    assert_eq!(controls["execution_holds"]["rows"][0]["job_id"], "next");
    assert_eq!(controls["mapping"]["rows"][0]["discord_thread_id"], 20);
    let pending = controls["unresolved_ingress"]["rows"].as_array().unwrap();
    assert_eq!(pending.len(), 2);
    assert!(
        pending
            .iter()
            .any(|r| r["ingress_id"] == "archive-request" && r["declared_action"] == "Archive")
    );
    assert!(
        pending
            .iter()
            .any(|r| r["ingress_id"] == "unbound-stop" && r["target_is_unbound"] == 1)
    );
    assert!(!report.to_string().contains("do not leak this prompt"));
    assert!(!report.to_string().contains("new input"));
    assert_eq!(queue::list(&path).unwrap(), jobs);
    assert_eq!(file_sha(&path), before);
}

#[tokio::test]
async fn archive_attempted_and_verified_are_reported_as_fences_not_unarchive_permission() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    fixture::dispatching(&path, "resident");
    queue::complete(&path, "origin").unwrap();
    let operation = cdr_store::archive_fence::reserve(
        &path,
        &std::collections::BTreeSet::from(["thread-b".into()]),
        None,
    )
    .unwrap();
    for phase in ["attempted", "verified"] {
        if phase == "verified" {
            cdr_store::archive_fence::verified(&path, &operation).unwrap();
        }
        let before = file_sha(&path);
        let report = inspect(&path, &[]).await.unwrap();
        assert_eq!(
            report["lifecycle_evidence"]["archive_fences"]["rows"][0]["phase"],
            phase
        );
        assert_eq!(
            report["lifecycle_evidence"]["state_labels_are_not_authority"],
            true
        );
        assert_eq!(report["execution_authorized"], false);
        assert_eq!(file_sha(&path), before);
    }
}

#[tokio::test]
async fn incident_policy_inventory_never_becomes_a_publishing_authorization() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    open_initialized(&path).unwrap();
    cdr_store::async_resolution::install_reviewed_policy(&path).unwrap();
    let before = file_sha(&path);
    let args = vec![
        OsString::from("inspect-async-recovery"),
        OsString::from("--database"),
        path.as_os_str().to_owned(),
        OsString::from("--thread-id"),
        OsString::from(cdr_store::async_resolution::REVIEWED_INCIDENT_THREAD),
    ];
    let text = cdr_runtime::admin::run(args).await.unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        value["lifecycle_evidence"]["reviewed_incident_policy_required"],
        true
    );
    assert_eq!(
        value["lifecycle_evidence"]["recovery_policy"]["rows"][0]["policy"],
        "publishing_recovery"
    );
    assert_eq!(value["publication_authorized"], false);
    assert_eq!(
        value["lifecycle_evidence"]["automatic_resume_authorized"],
        false
    );
    assert_eq!(file_sha(&path), before);
}

#[tokio::test]
async fn bounded_control_inventory_marks_truncation_and_oversized_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    fixture::dispatching(&path, "resident");
    let mut db = open_initialized(&path).unwrap();
    let tx = db.transaction().unwrap();
    for index in 0..130 {
        tx.execute(
            "INSERT INTO cdr_stop_revision_receipts VALUES(?1,'thread-b',?2,?3)",
            rusqlite::params![
                format!("stop-{index:03}"),
                index + 1,
                if index == 0 {
                    "x".repeat(131_073)
                } else {
                    "private original input".into()
                }
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    drop(db);
    let before = file_sha(&path);
    let report = inspect(&path, &[]).await.unwrap();
    let section = &report["lifecycle_evidence"]["stop_receipts"];
    assert_eq!(section["rows"].as_array().unwrap().len(), 128);
    assert_eq!(section["truncated"], true);
    assert_eq!(section["rows"][0]["details_oversized"], true);
    assert!(section["rows"][0]["details_sha256"].is_null());
    assert!(!report.to_string().contains("private original input"));
    assert_eq!(report["lifecycle_evidence"]["absence_is_clearance"], false);
    assert_eq!(file_sha(&path), before);
}

#[tokio::test]
async fn missing_control_extensions_remain_unknown_without_creating_tables() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(
        None,
        "user_version",
        cdr_store::schema::LATEST_STORE_SCHEMA_VERSION,
    )
    .unwrap();
    drop(db);
    let before = file_sha(&path);
    let value = inspect(&path, &[]).await.unwrap();
    for section in [
        "mapping",
        "recovery_policy",
        "stop_receipts",
        "stop_controls",
        "archive_fences",
        "execution_holds",
        "unresolved_ingress",
    ] {
        assert_eq!(
            value["lifecycle_evidence"][section]["schema_present"], false,
            "{section}"
        );
    }
    assert_eq!(value["lifecycle_evidence"]["absence_is_clearance"], false);
    assert_eq!(file_sha(&path), before);
}

#[tokio::test]
async fn malformed_control_schema_is_an_error_not_an_empty_clearance() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("broken.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(
        None,
        "user_version",
        cdr_store::schema::LATEST_STORE_SCHEMA_VERSION,
    )
    .unwrap();
    db.execute_batch("CREATE TABLE cdr_stop_controls(wrong TEXT)")
        .unwrap();
    drop(db);
    let before = file_sha(&path);
    assert!(inspect(&path, &[]).await.is_err());
    assert_eq!(file_sha(&path), before);
}
