use cdr_app_server::AppServerClient;
use cdr_runtime::restart_readiness::maintenance::{AbsentTarget, check_absent_maintenance};
use cdr_store::queue::{
    NewAppServerForkHandoff, begin_app_server_fork_handoff, mark_app_server_managed_target,
};
use rusqlite::Connection;
use std::time::Duration;
#[allow(dead_code)]
#[path = "support/restart_readiness.rs"]
mod support;

#[tokio::test]
async fn same_id_intake_managed_and_fork_are_not_absence_exceptions() {
    for case in ["intake", "managed", "fork", "late_managed"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let state = temp.path().join("state.sqlite");
        Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        support::seed_target(&db);
        match case {
            "intake" => {
                Connection::open(&db).unwrap().execute("INSERT INTO codex_prompt_intakes (job_id,target_thread_id,channel_id,raw_prompt,auto_queue_when_busy,require_current_mirror,created_at,updated_at) VALUES ('intake','bot-thread',71,'preserve',0,0,1,1)", []).unwrap();
            }
            "managed" => mark_app_server_managed_target(&db, "bot-thread", 1).unwrap(),
            "fork" => {
                begin_app_server_fork_handoff(
                    &db,
                    NewAppServerForkHandoff {
                        handoff_id: "handoff",
                        ambiguous_job_id: None,
                        source_thread_id: "bot-thread",
                        expected_generation: 1,
                        quarantine_reason: "fixture",
                    },
                )
                .unwrap();
            }
            _ => {
                cdr_store::mapping::upsert_thread(&db, "other", "p", "other", 70, 72, 1.0).unwrap();
            }
        }
        let before = std::fs::read(&db).unwrap();
        let ticket = AbsentTarget {
            state_db: state,
            thread_id: "bot-thread".into(),
            room: 71,
            parent: 70,
        };
        let mut config = support::fake_config(
            if case == "late_managed" { case } else { "idle" },
            &temp.path().join("methods"),
        );
        config.environment.insert(
            "CDR_MAINTENANCE_FIXTURE_DB".into(),
            db.to_string_lossy().into(),
        );
        let client = AppServerClient::start(config).await.unwrap();
        let result = check_absent_maintenance(
            &db,
            &client,
            Duration::ZERO,
            Duration::from_secs(2),
            &ticket,
        )
        .await;
        client.close().await.unwrap();
        assert!(result.is_err(), "{case}: {result:?}");
        if case != "late_managed" {
            assert_eq!(before, std::fs::read(&db).unwrap(), "{case} mutated DB");
        }
    }
}
