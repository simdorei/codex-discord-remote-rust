#[path = "support/mutation_isolation.rs"]
mod support;

#[path = "mutation_isolation/faults.rs"]
mod faults;

use cdr_app_server::ResidentAppServer;
use rusqlite::Connection;
use serde_json::json;
use support::{EXPIRY, WAIT, count, seen, start};

const MUTATE: &str = "thread/settings/update";

#[tokio::test]
async fn p03b_scoped_timeout_does_not_quarantine_b() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = start(&db, &log, "runtime-a", "normal").await;
    let original = server
        .request(
            MUTATE,
            json!({"threadId":"thread-a","fixture":"hold"}),
            EXPIRY,
            Some(1),
        )
        .await;
    let b = server
        .request(MUTATE, json!({"threadId":"thread-b"}), WAIT, Some(1))
        .await;
    let again = server
        .request(MUTATE, json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await;
    let healthy = server.lifecycle_snapshot().await.healthy;
    server.close().await.unwrap();
    assert!(original.is_err());
    assert!(
        b.is_ok(),
        "a fully flushed scoped timeout incorrectly fenced B: {b:?}"
    );
    assert!(healthy, "clean scoped ambiguity is not transport damage");
    assert!(
        again.is_err(),
        "unresolved A must not be issued a second mutation"
    );
    assert_eq!(count(&log, MUTATE, "thread-a"), 1);
    assert_eq!(count(&log, MUTATE, "thread-b"), 1);
}

#[tokio::test]
async fn p03b_cancel_and_late_duplicates_preserve_waiting_b_and_hold_a() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = start(&db, &log, "runtime-a", "normal").await;
    let a_server = server.clone();
    let a = tokio::spawn(async move {
        a_server
            .request(
                MUTATE,
                json!({"threadId":"thread-a","fixture":"hold"}),
                WAIT,
                Some(1),
            )
            .await
    });
    seen(&log, MUTATE, "thread-a").await;
    server
        .request("thread/read", json!({"threadId":"barrier"}), WAIT, Some(1))
        .await
        .unwrap();
    a.abort();
    assert!(a.await.unwrap_err().is_cancelled());
    // The child emits late A only after it receives B, so B is already pending.
    let b = server
        .request(
            MUTATE,
            json!({"threadId":"thread-b","fixture":"late-a"}),
            WAIT,
            Some(1),
        )
        .await;
    let again = server
        .request(MUTATE, json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await;
    server.close().await.unwrap();
    assert!(b.is_ok(), "caller cancellation incorrectly fenced B: {b:?}");
    assert_eq!(b.unwrap()["marker"], "thread-b");
    assert!(
        again.is_err(),
        "late or duplicate A is not replay permission"
    );
    assert_eq!(count(&log, MUTATE, "thread-a"), 1);
}

#[tokio::test]
async fn p03b_unknown_attempt_survives_cold_owner() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let first = start(&db, &log, "runtime-old", "normal").await;
    let _ = first
        .request(
            MUTATE,
            json!({"threadId":"thread-a","fixture":"hold"}),
            EXPIRY,
            Some(1),
        )
        .await;
    first.close().await.unwrap();
    drop(first);
    let cold = start(&db, &log, "runtime-new", "normal").await;
    let a = cold
        .request(MUTATE, json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await;
    let b = cold
        .request(MUTATE, json!({"threadId":"thread-b"}), WAIT, Some(1))
        .await;
    cold.close().await.unwrap();
    assert!(a.is_err(), "cold owner lost the original unknown mutation");
    assert!(
        b.is_ok(),
        "old A evidence must not blanket-hold independent B"
    );
    assert_eq!(count(&log, MUTATE, "thread-a"), 1);
    assert_eq!(count(&log, MUTATE, "thread-b"), 1);
}

#[tokio::test]
async fn p03b_pre_dispatch_evidence_exists_while_ack_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = start(&db, &log, "runtime-a", "normal").await;
    let a_server = server.clone();
    let a = tokio::spawn(async move {
        a_server
            .request(
                MUTATE,
                json!({"threadId":"thread-a","fixture":"hold"}),
                WAIT,
                Some(1),
            )
            .await
    });
    seen(&log, MUTATE, "thread-a").await;
    let db = Connection::open(db).unwrap();
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='codex_mutation_attempts')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let pending = if exists {
        db.query_row("SELECT count(*) FROM codex_mutation_attempts WHERE target_thread_id='thread-a' AND state='prepared'", [], |r| r.get::<_,i64>(0)).unwrap()
    } else {
        0
    };
    a.abort();
    let _ = a.await;
    server.close().await.unwrap();
    assert_eq!(
        pending, 1,
        "the child received bytes without durable pre-dispatch evidence"
    );
}

#[tokio::test]
async fn p03b_unknown_and_create_stay_global_across_cold_owner() {
    for method in ["thread/start", "unregistered/mutate"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let first = start(&db, &log, "runtime-old", "normal").await;
        let _ = first
            .request(method, json!({"fixture":"hold"}), EXPIRY, Some(1))
            .await;
        let quarantined = first.lifecycle_snapshot().await.quarantined;
        first.close().await.unwrap();
        drop(first);
        let cold = start(&db, &log, "runtime-new", "normal").await;
        let b = cold
            .request(MUTATE, json!({"threadId":"thread-b"}), WAIT, Some(1))
            .await;
        cold.close().await.unwrap();
        assert!(
            quarantined,
            "unknown-scope request cannot receive read-only isolation"
        );
        assert!(
            b.is_err(),
            "cold startup erased unresolved global scope for {method}"
        );
        assert_eq!(count(&log, MUTATE, "thread-b"), 0);
    }
}

#[tokio::test]
async fn p03b_partial_transport_damage_keeps_evidence_and_b_recovers() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let first = start(&db, &log, "runtime-old", "partial").await;
    let result = first
        .request(
            "turn/start",
            json!({"threadId":"thread-a","input":[{"type":"text","text":"x".repeat(262_144)}]}),
            WAIT,
            Some(1),
        )
        .await;
    let healthy = first.lifecycle_snapshot().await.healthy;
    first.close().await.unwrap();
    drop(first);
    let cold = start(&db, &log, "runtime-new", "normal").await;
    let a = cold
        .request(MUTATE, json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await;
    let b = cold
        .request(MUTATE, json!({"threadId":"thread-b"}), WAIT, Some(1))
        .await;
    cold.close().await.unwrap();
    assert!(result.is_err());
    assert!(
        !healthy,
        "partial write must retain shared transport protection"
    );
    assert!(
        support::calls(&log)
            .iter()
            .any(|r| r["method"] == "fixture/partial")
    );
    assert!(a.is_err(), "physical loss cannot erase A's dispatch intent");
    assert!(b.is_ok());
}

#[tokio::test]
async fn p03b_legacy_without_journal_remains_conservative() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = ResidentAppServer::start(support::config(&log, "normal"))
        .await
        .unwrap();
    let _ = server
        .request(
            MUTATE,
            json!({"threadId":"thread-a","fixture":"hold"}),
            EXPIRY,
            Some(1),
        )
        .await;
    let quarantined = server.lifecycle_snapshot().await.quarantined;
    server.close().await.unwrap();
    assert!(
        quarantined,
        "absence of a durable adapter cannot grant target isolation"
    );
}
