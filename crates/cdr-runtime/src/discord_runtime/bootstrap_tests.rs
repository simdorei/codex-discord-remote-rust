use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use cdr_app_server::{AppServerError, ResidentLifecycleSnapshot};
use cdr_store::mapping::upsert_thread;
use cdr_store::processed::{claim, mark};
use cdr_store::queue::{
    NewAppServerForkHandoff, begin_app_server_fork_handoff, complete_app_server_fork_handoff,
};
use cdr_store::schema::open_initialized;
use rusqlite::Connection;

use super::{prepare_storage, reconcile_bridge_state};
use crate::bridge_state::{BridgeState, SavedThreadSettings};
use crate::queue_recovery_transport::{QueueRecoveryServer, stabilize_after_queue_recovery};
use crate::runtime_paths::{PathSource, RuntimePaths};

const FAR_FUTURE: f64 = 10_000_000_000.0;

struct QuarantinedRecoveryServer {
    restart_calls: AtomicUsize,
}

impl QueueRecoveryServer for QuarantinedRecoveryServer {
    async fn recovery_snapshot(&self) -> ResidentLifecycleSnapshot {
        ResidentLifecycleSnapshot {
            generation: 1,
            healthy: false,
            quarantined: true,
            restart_pending: true,
            process_id: Some(7),
        }
    }

    async fn force_recovery_restart(&self) -> Result<bool, AppServerError> {
        self.restart_calls.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}

#[tokio::test]
async fn quarantined_queue_recovery_refreshes_transport_before_bootstrap_continues() {
    let server = QuarantinedRecoveryServer {
        restart_calls: AtomicUsize::new(0),
    };

    assert!(
        stabilize_after_queue_recovery(&server)
            .await
            .expect("restart quarantined recovery transport")
    );
    assert_eq!(server.restart_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn processed_claims_survive_prepare_storage_regardless_of_age_or_mark_state() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let paths = test_paths(temp.path());
    let _state = Connection::open(&paths.state_db).expect("create readable state database");

    assert!(claim(&paths.mirror_db, 701, 1.0).expect("insert ancient claim-only row"));
    assert!(claim(&paths.mirror_db, 702, 2.0).expect("insert ancient marked row"));
    mark(&paths.mirror_db, 702, 3.0).expect("mark ancient row");

    prepare_storage(&paths).expect("prepare storage without expiring replay barriers");

    let reopened = open_initialized(&paths.mirror_db).expect("reopen prepared mirror store");
    let retained: i64 = reopened
        .query_row(
            "SELECT COUNT(*) FROM discord_processed_messages WHERE message_id IN (701, 702)",
            [],
            |row| row.get(0),
        )
        .expect("count retained replay barriers");
    assert_eq!(
        retained, 2,
        "prepare_storage must retain both replay barriers"
    );
    drop(reopened);

    assert!(!claim(&paths.mirror_db, 701, FAR_FUTURE).expect("reject claim-only duplicate"));
    assert!(!claim(&paths.mirror_db, 702, FAR_FUTURE).expect("reject marked duplicate"));
}

#[test]
fn startup_reconciles_bridge_state_after_sqlite_finalize_preceded_a_crash() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let paths = test_paths(temp.path());
    fs::create_dir_all(paths.bridge_state.parent().unwrap()).unwrap();
    fs::write(
        &paths.bridge_state,
        r#"{"selected_thread_id":"source","thread_settings":{"source":{"model":"model-a","future":7},"settings-only":{"reasoning":"max"}},"future":{"keep":true}}"#,
    )
    .unwrap();
    upsert_thread(&paths.mirror_db, "source", "project", "Title", 9, 10, 1.0).unwrap();
    complete_handoff(&paths.mirror_db, "handoff-1", "source", "middle");
    complete_handoff(&paths.mirror_db, "handoff-2", "middle", "target");
    complete_handoff(
        &paths.mirror_db,
        "handoff-settings",
        "settings-only",
        "settings-target",
    );

    let restarted = BridgeState::new(paths.bridge_state.clone());
    reconcile_bridge_state(&restarted, &paths.mirror_db).unwrap();
    let first_reconciliation = fs::read(&paths.bridge_state).unwrap();
    reconcile_bridge_state(&restarted, &paths.mirror_db).unwrap();

    assert_eq!(
        restarted.selected_thread_id().unwrap().as_deref(),
        Some("target")
    );
    assert_eq!(
        restarted.thread_settings("target").unwrap(),
        SavedThreadSettings {
            model: Some("model-a".into()),
            reasoning: None,
            speed: None,
        }
    );
    assert_eq!(
        restarted
            .thread_settings("settings-target")
            .unwrap()
            .reasoning
            .as_deref(),
        Some("max")
    );
    assert_eq!(fs::read(&paths.bridge_state).unwrap(), first_reconciliation);
    let json: serde_json::Value = serde_json::from_slice(&first_reconciliation).unwrap();
    assert_eq!(json["thread_settings"]["source"]["future"], 7);
    assert_eq!(json["future"]["keep"], true);
}

#[test]
fn held_tracked_source_or_alias_does_not_block_independent_bootstrap_reconciliation() {
    use cdr_store::dead_generation::{
        DeadGenerationCapture, activate_runtime, capture_dead_generation,
    };

    for held in ["source", "middle", "target"] {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        fs::create_dir_all(paths.bridge_state.parent().unwrap()).unwrap();
        fs::write(&paths.bridge_state,
            r#"{"selected_thread_id":"source","thread_settings":{"source":{"model":"preserve"},"other":{"reasoning":"max"}}}"#
        ).unwrap();
        activate_runtime(&paths.mirror_db, "runtime-a").unwrap();
        if held != "source" {
            complete_handoff(&paths.mirror_db, "one", "source", "middle");
            complete_handoff(&paths.mirror_db, "two", "middle", "target");
        }
        complete_handoff(&paths.mirror_db, "independent", "other", "other-target");
        capture_dead_generation(
            &paths.mirror_db,
            DeadGenerationCapture {
                runtime_id: "runtime-a",
                generation: 1,
                snapshot_json: "{}",
                affected_targets: &[held.into()],
                startup_channel_id: Some(88),
                has_unscoped_requests: false,
                now: 2.0,
            },
        )
        .unwrap();
        let bridge = BridgeState::new(paths.bridge_state.clone());

        reconcile_bridge_state(&bridge, &paths.mirror_db)
            .expect("one held source must not abort all startup reconciliation");

        assert_eq!(
            bridge.selected_thread_id().unwrap().as_deref(),
            Some("source")
        );
        assert_eq!(
            bridge.thread_settings("source").unwrap().model.as_deref(),
            Some("preserve")
        );
        assert_eq!(
            bridge
                .thread_settings("other-target")
                .unwrap()
                .reasoning
                .as_deref(),
            Some("max")
        );
    }
}

#[test]
fn actual_store_failure_remains_fatal_during_bootstrap_reconciliation() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let bridge = BridgeState::new(paths.bridge_state.clone());
    bridge.set_selected_thread_id(Some("source")).unwrap();
    // The database path is a directory, not an available SQLite store.
    fs::create_dir_all(&paths.mirror_db).unwrap();
    assert!(reconcile_bridge_state(&bridge, &paths.mirror_db).is_err());
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("source")
    );
}

fn complete_handoff(path: &Path, handoff_id: &str, source: &str, target: &str) {
    begin_app_server_fork_handoff(
        path,
        NewAppServerForkHandoff {
            handoff_id,
            ambiguous_job_id: None,
            source_thread_id: source,
            expected_generation: 1,
            quarantine_reason: "test",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(path, handoff_id, target, 1).unwrap();
}

fn test_paths(root: &Path) -> RuntimePaths {
    let codex_home = root.join("codex-home");
    RuntimePaths {
        root: root.to_path_buf(),
        codex_home: codex_home.clone(),
        mirror_db: root.join("mirror.sqlite"),
        state_db: root.join("state.sqlite"),
        bridge_state: codex_home.join("bridge-state.json"),
        log_db: codex_home.join("logs.sqlite"),
        global_state: codex_home.join("global-state.json"),
        session_index: codex_home.join("session-index.jsonl"),
        archived_sessions: codex_home.join("archived-sessions"),
        maintenance_backup_root: codex_home.join("maintenance-backups"),
        attachment_dir: root.join("attachments"),
        codex_exe: PathBuf::from("unused-codex.exe"),
        codex_exe_source: PathSource::Environment,
    }
}
