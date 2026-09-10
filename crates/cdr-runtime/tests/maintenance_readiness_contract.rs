use cdr_app_server::AppServerClient;
use cdr_runtime::restart_readiness::maintenance::{AbsentTarget, check_absent_maintenance};
use cdr_runtime::restart_readiness::{RestartReadinessState, check_restart_readiness};
use cdr_store::mapping::upsert_thread;
use rusqlite::Connection;
use std::time::Duration;
#[path = "support/restart_readiness.rs"]
mod support;

#[test]
fn only_the_known_new_optional_table_may_be_absent_in_legacy_preflight() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    support::seed_target(&db);
    let connection = Connection::open(&db).unwrap();
    connection
        .execute("DROP TABLE codex_commentary_outbox", [])
        .unwrap();
    let before = std::fs::read(&db).unwrap();
    assert_eq!(
        cdr_store::room_cleanup::pending_reason_pre_commentary_schema(&db, 71, Some("bot-thread"))
            .unwrap(),
        None
    );
    assert_eq!(before, std::fs::read(&db).unwrap());
    assert!(
        cdr_store::room_cleanup::pending_reason(&db, 71, Some("bot-thread")).is_err(),
        "normal cleanup retains strict schema checks"
    );
    connection
        .execute("DROP TABLE codex_delivery_outbox", [])
        .unwrap();
    assert!(
        cdr_store::room_cleanup::pending_reason_pre_commentary_schema(&db, 71, Some("bot-thread"))
            .is_err()
    );
}

#[tokio::test]
async fn maintenance_never_waives_pending_work_for_absent_id() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let state = temp.path().join("state.sqlite");
    Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    support::seed_target(&db);
    cdr_store::queue::enqueue(
        &db,
        cdr_store::queue::NewQueueJob {
            job_id: "pending",
            target_thread_id: "bot-thread",
            channel_id: 71,
            owner_user_id: None,
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "must not disappear",
            queued: true,
            ack_sent: false,
            created_at: 1.0,
        },
    )
    .unwrap();
    let ticket = AbsentTarget {
        state_db: state,
        thread_id: "bot-thread".into(),
        room: 71,
        parent: 70,
    };
    let client = AppServerClient::start(support::fake_config("idle", &temp.path().join("methods")))
        .await
        .unwrap();
    assert!(
        check_absent_maintenance(
            &db,
            &client,
            Duration::ZERO,
            Duration::from_secs(2),
            &ticket
        )
        .await
        .is_err()
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn maintenance_exception_is_exact_and_ordinary_readiness_still_fails() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let state = temp.path().join("state.sqlite");
    Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    support::seed_target(&db);
    let ticket = AbsentTarget {
        state_db: state,
        thread_id: "bot-thread".into(),
        room: 71,
        parent: 70,
    };
    let client = AppServerClient::start(support::fake_config(
        "server_failure",
        &temp.path().join("methods"),
    ))
    .await
    .unwrap();
    let before = std::fs::read(&db).unwrap();
    assert_eq!(
        check_absent_maintenance(
            &db,
            &client,
            Duration::ZERO,
            Duration::from_secs(2),
            &ticket
        )
        .await
        .unwrap(),
        RestartReadinessState::Ready
    );
    assert_eq!(
        before,
        std::fs::read(&db).unwrap(),
        "preflight must be read-only"
    );
    assert!(
        check_restart_readiness(&db, &client, Duration::ZERO, Duration::from_secs(2))
            .await
            .is_err()
    );
    upsert_thread(&db, "other", "p", "other", 70, 72, 1.0).unwrap();
    assert!(
        check_absent_maintenance(
            &db,
            &client,
            Duration::ZERO,
            Duration::from_secs(2),
            &ticket
        )
        .await
        .is_err(),
        "other RPC failures must not be waived"
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn maintenance_rejects_active_shared_changed_and_failed_inventory() {
    for case in ["active", "shared", "parent", "inventory"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let state = temp.path().join("state.sqlite");
        let source = Connection::open(&state).unwrap();
        source
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        upsert_thread(&db, "absent", "p", "absent", 70, 71, 1.0).unwrap();
        if case == "active" {
            source
                .execute(
                    "UPDATE threads SET id='absent',rollout_path='missing' WHERE id='thread-a'",
                    [],
                )
                .unwrap();
        }
        if case == "shared" {
            upsert_thread(&db, "other", "p", "shared", 70, 71, 1.0).unwrap();
        }
        if case == "inventory" {
            source
                .execute("ALTER TABLE threads RENAME TO broken", [])
                .unwrap();
        }
        let ticket = AbsentTarget {
            state_db: state,
            thread_id: "absent".into(),
            room: 71,
            parent: if case == "parent" { 99 } else { 70 },
        };
        let client =
            AppServerClient::start(support::fake_config("idle", &temp.path().join("methods")))
                .await
                .unwrap();
        assert!(
            check_absent_maintenance(
                &db,
                &client,
                Duration::ZERO,
                Duration::from_secs(2),
                &ticket
            )
            .await
            .is_err(),
            "{case}"
        );
        client.close().await.unwrap();
    }
}
