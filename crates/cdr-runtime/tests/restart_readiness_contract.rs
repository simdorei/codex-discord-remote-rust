use std::ffi::OsString;
use std::time::Duration;

use cdr_runtime::restart_readiness::{
    RestartReadinessError, RestartReadinessOptions, RestartReadinessState,
    wait_for_restart_with_config,
};
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, mark_running};
use rusqlite::Connection;

#[path = "support/restart_readiness.rs"]
mod support;

use support::{check_scenario, fake_config, read_methods, seed_target};

const QUICK: Duration = Duration::from_millis(40);

#[test]
fn cli_parses_restart_readiness_time_bounds() {
    let args = cdr_runtime::startup::StartupArgs::parse([
        OsString::from("--restart-readiness"),
        OsString::from("--restart-quiet-seconds"),
        OsString::from("17"),
        OsString::from("--restart-wait-timeout-seconds"),
        OsString::from("23"),
    ])
    .unwrap();
    assert!(args.restart_readiness);
    assert_eq!(args.restart_quiet_seconds, 17);
    assert_eq!(args.restart_wait_timeout_seconds, 23);
}

#[test]
fn cli_rejects_missing_or_invalid_restart_time_bounds() {
    for arguments in [
        vec![OsString::from("--restart-quiet-seconds")],
        vec![
            OsString::from("--restart-wait-timeout-seconds"),
            OsString::from("not-a-number"),
        ],
    ] {
        assert!(cdr_runtime::startup::StartupArgs::parse(arguments).is_err());
    }
}

#[tokio::test]
async fn idle_bot_owned_thread_is_ready_via_read_only_thread_read() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_target(&db);

    let (state, methods) = check_scenario(&db, "idle", Duration::ZERO, QUICK)
        .await
        .unwrap();

    assert_eq!(state, RestartReadinessState::Ready);
    assert!(methods.iter().any(|method| method == "thread/read"));
    assert!(methods.iter().all(|method| matches!(
        method.as_str(),
        "initialize" | "initialized" | "thread/read"
    )));
}

#[tokio::test]
async fn active_thread_is_blocked() {
    let (state, _) = scenario("busy", Duration::ZERO).await.unwrap();
    assert!(
        matches!(state, RestartReadinessState::Blocked { ref reason } if reason.contains("active"))
    );
}

#[tokio::test]
async fn waiting_approval_and_user_input_are_blocked() {
    for (scenario_name, expected) in [
        ("approval", "waitingOnApproval"),
        ("input", "waitingOnUserInput"),
    ] {
        let (state, _) = scenario(scenario_name, Duration::ZERO).await.unwrap();
        assert!(
            matches!(state, RestartReadinessState::Blocked { ref reason } if reason.contains(expected))
        );
    }
}

#[tokio::test]
async fn recently_updated_idle_thread_is_blocked() {
    let (state, _) = scenario("recent", Duration::from_secs(90)).await.unwrap();
    assert!(
        matches!(state, RestartReadinessState::Blocked { ref reason } if reason.contains("recent"))
    );
}

#[tokio::test]
async fn malformed_or_unknown_thread_state_is_an_actual_error() {
    for scenario_name in ["malformed", "mystery"] {
        let error = scenario(scenario_name, Duration::ZERO).await.unwrap_err();
        assert!(matches!(
            error,
            RestartReadinessError::InvalidThreadState { .. }
        ));
    }
}

#[tokio::test]
async fn app_server_request_timeout_is_an_actual_error() {
    let error = scenario("timeout", Duration::ZERO).await.unwrap_err();
    assert!(matches!(error, RestartReadinessError::AppServer(_)));
}

#[tokio::test]
async fn app_server_failure_is_an_actual_error() {
    let error = scenario("server_failure", Duration::ZERO)
        .await
        .unwrap_err();
    assert!(matches!(error, RestartReadinessError::AppServer(_)));
}

#[tokio::test]
async fn wait_loop_eventually_observes_ready_and_closes_the_server() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let methods = temp.path().join("methods.txt");
    seed_target(&db);
    wait_for_restart_with_config(
        &db,
        fake_config("eventual", &methods),
        options(Duration::from_millis(300)),
    )
    .await
    .unwrap();
    assert_eq!(
        read_methods(&methods)
            .iter()
            .filter(|method| method.as_str() == "thread/read")
            .count(),
        2
    );
}

#[tokio::test]
async fn wait_loop_honors_its_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let methods = temp.path().join("methods.txt");
    seed_target(&db);
    let error = wait_for_restart_with_config(
        &db,
        fake_config("busy", &methods),
        options(Duration::from_millis(55)),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        RestartReadinessError::DeadlineExceeded { .. }
    ));
}

#[tokio::test]
async fn durable_starting_and_running_jobs_block_even_if_thread_read_is_idle() {
    for running in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        seed_target(&db);
        enqueue(&db, queue_job()).unwrap();
        let claimed = begin_attempt(&db, "job", &[], 1).unwrap();
        if running {
            mark_running(&db, "job", "turn", 1).unwrap();
        }
        let (state, _) = check_scenario(&db, "idle", Duration::ZERO, QUICK)
            .await
            .unwrap();
        let expected = if running { "running" } else { "starting" };
        assert!(
            matches!(state, RestartReadinessState::Blocked { ref reason } if reason.contains(expected))
        );
        drop(claimed);
    }
}

#[tokio::test]
async fn recoverable_queue_states_do_not_permanently_block_restart() {
    for (state, turn_id, last_error) in [
        ("pending", None, ""),
        (
            "starting",
            None,
            "[cdr-rust:turn-start-candidates-ambiguous:v1] candidate_count=1",
        ),
        (
            "running",
            Some("cdr-quarantined:handoff"),
            "[cdr-rust:app-server-fork-quarantine:v1] uncertain start",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        seed_target(&db);
        enqueue(&db, queue_job()).unwrap();
        Connection::open(&db)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET state = ?, turn_id = ?, last_error = ? WHERE job_id = 'job'",
                rusqlite::params![state, turn_id, last_error],
            )
            .unwrap();
        let (readiness, _) = check_scenario(&db, "idle", Duration::ZERO, QUICK)
            .await
            .unwrap();
        assert_eq!(readiness, RestartReadinessState::Ready, "state={state}");
    }
}

async fn scenario(
    name: &str,
    quiet: Duration,
) -> Result<(RestartReadinessState, Vec<String>), RestartReadinessError> {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_target(&db);
    check_scenario(&db, name, quiet, QUICK).await
}

fn options(wait_timeout: Duration) -> RestartReadinessOptions {
    RestartReadinessOptions {
        quiet: Duration::ZERO,
        wait_timeout,
        poll_interval: Duration::from_millis(10),
        request_timeout: QUICK,
        startup_timeout: Duration::from_secs(2),
        close_timeout: Duration::from_secs(2),
    }
}

fn queue_job() -> NewQueueJob<'static> {
    NewQueueJob {
        job_id: "job",
        target_thread_id: "bot-thread",
        channel_id: 71,
        owner_user_id: Some(7),
        discord_message_id: Some(8),
        app_server_generation: 1,
        prompt: "prompt",
        queued: false,
        ack_sent: true,
        created_at: 1.0,
    }
}
