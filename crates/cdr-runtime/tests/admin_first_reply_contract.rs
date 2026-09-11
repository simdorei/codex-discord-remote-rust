use rusqlite::Connection;
use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn run(root: &Path, database: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--admin", "inspect-new-first-reply", "--repo-root"])
        .arg(root)
        .arg("--database")
        .arg(database)
        .args(["--job-id", "job"])
        .env_clear()
        .output()
        .unwrap()
}

fn seed(path: &Path) {
    Connection::open(path).unwrap().execute_batch(r#"
      CREATE TABLE discord_ingress_journal(ingress_id,kind,event_id,channel_id,target_thread_id,outcome_json,confirmation_delivered,owner_id);
      INSERT INTO discord_ingress_journal VALUES('message:1','message',1,42,'thread','{"new_verification":{"thread_id":"thread","channel_id":43}}',0,'job');
      CREATE TABLE codex_delivery_outbox(job_id,target_thread_id,turn_id,channel_id,content);
      INSERT INTO codex_delivery_outbox VALUES('job','thread','turn',43,'private final 한글');
      CREATE TABLE mirror_threads(codex_thread_id,discord_thread_id);
      INSERT INTO mirror_threads VALUES('thread',43);
      CREATE TABLE codex_delivery_receipts(receipt_key,message_id,retryable,blocked_reason);
    "#).unwrap();
}

#[test]
fn missing_database_is_reported_and_not_created() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("missing.sqlite");
    let out = run(root.path(), &path);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Cannot inspect first reply"));
    assert!(!path.exists());
}

#[test]
fn preview_never_authorizes_delivery_or_replay_and_changes_no_bytes() {
    for receipt in [
        "absent",
        "confirmed",
        "rejected_blocked",
        "definite_rejection",
        "unknown",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mirror.sqlite");
        seed(&path);
        if receipt != "absent" {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute(
                    "INSERT INTO codex_delivery_receipts VALUES(?1,?2,?3,?4)",
                    rusqlite::params![
                        r#"[42,"message/reply/v1","inbound-message/1/action-result",0]"#,
                        (receipt == "confirmed").then_some(900_i64),
                        i64::from(receipt == "definite_rejection"),
                        (receipt == "rejected_blocked").then_some("synthetic failure")
                    ],
                )
                .unwrap();
        }
        let before = fs::read(&path).unwrap();
        let out = run(root.path(), &path);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8(out.stdout).unwrap();
        let report: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(report["normal_ack_receipt"], receipt);
        assert_eq!(report["destination_matches"], true);
        assert_eq!(report["delivery_authorized"], false);
        assert_eq!(report["replay_authorized"], false);
        assert_eq!(report["read_only"], true);
        assert_eq!(report["final_sha256"].as_str().unwrap().len(), 64);
        assert!(!text.contains("private final") && !text.contains("synthetic failure"));
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn duplicate_or_wrong_destination_remains_unauthorized() {
    for mutation in [
        "INSERT INTO codex_delivery_outbox SELECT * FROM codex_delivery_outbox",
        "UPDATE mirror_threads SET codex_thread_id='other'",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mirror.sqlite");
        seed(&path);
        Connection::open(&path)
            .unwrap()
            .execute(mutation, [])
            .unwrap();
        let before = fs::read(&path).unwrap();
        let out = run(root.path(), &path);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_ne!(report["destination_matches"], true);
        assert_eq!(report["delivery_authorized"], false);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
