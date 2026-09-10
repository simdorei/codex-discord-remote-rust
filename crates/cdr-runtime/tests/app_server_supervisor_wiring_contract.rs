const DISCORD_RUNTIME: &str = include_str!("../src/discord_runtime.rs");
const WORKER_SUPERVISION: &str = include_str!("../src/discord_runtime/worker_supervision.rs");

fn run_body() -> &'static str {
    let start = DISCORD_RUNTIME
        .find("pub async fn run(")
        .expect("discord runtime run function");
    let end = DISCORD_RUNTIME[start..]
        .find("async fn start_app_server(")
        .map(|offset| start + offset)
        .expect("start_app_server boundary after run");
    &DISCORD_RUNTIME[start..end]
}

fn position(source: &str, needle: &str) -> usize {
    source
        .find(needle)
        .unwrap_or_else(|| panic!("missing runtime wiring marker: {needle}"))
}

#[test]
fn supervisor_is_monitored_once_after_all_fallible_startup_steps() {
    let body = run_body();
    let typed_start = position(body, "TypedIngressWorkers::start(");
    let startup_complete = typed_start + position(&body[typed_start..], ".await?;");
    let supervisor = position(body, "\"app-server-supervisor\"");
    let active_wait = position(body, "let shutdown_cause = tokio::select!");

    assert_eq!(body.matches(".run_restart_supervisor(").count(), 1);
    assert_eq!(body.matches("\"app-server-supervisor\"").count(), 1);
    assert!(startup_complete < supervisor);
    assert!(supervisor < active_wait);
    assert!(body[supervisor..active_wait].contains("spawn_unit_worker("));
    assert!(
        !body[(startup_complete + ".await?;".len())..active_wait].contains(".await?"),
        "a fallible startup await after monitor ownership could detach workers"
    );
}

#[test]
fn shutdown_keeps_heartbeat_alive_until_server_close_then_propagates() {
    let body = run_body();
    let main_signal = position(body, "completion_shutdown.send(true)");
    let worker_join = position(body, "workers.shutdown_for_cause(");
    let close = position(body, "complete_before_shutdown_deadline(");
    let heartbeat_signal = position(body, "heartbeat_shutdown.send(true)");
    let heartbeat_join = position(body, "heartbeat.shutdown_until(shutdown_deadline)");
    let propagate = position(body, "shutdown_cause.propagate(");

    assert!(main_signal < worker_join);
    assert!(worker_join < close);
    assert!(close < heartbeat_signal);
    assert!(heartbeat_signal < heartbeat_join);
    assert!(heartbeat_join < propagate);
    assert!(body[main_signal..propagate].contains("tokio::join!("));
    assert!(
        !body[main_signal..close].contains("?;"),
        "cleanup errors must be stored until server.close has run"
    );
}

#[test]
fn monitored_worker_set_drains_then_aborts_and_joins_by_one_deadline() {
    let compact: String = WORKER_SUPERVISION.split_whitespace().collect();

    assert!(compact.contains("timeout_at(drain_deadline,worker.join())"));
    assert!(compact.contains("forworkerin&self.workers{worker.abort();}"));
    assert!(compact.contains("timeout_at(deadline,worker.join())"));
    assert!(WORKER_SUPERVISION.contains("terminate_on_shutdown_timeout"));
    assert!(compact.contains("source.is_cancelled()"));
    assert!(WORKER_SUPERVISION.contains("WorkerShutdownTimeout"));
    assert!(WORKER_SUPERVISION.contains("WorkerTask { worker, source }"));
}
