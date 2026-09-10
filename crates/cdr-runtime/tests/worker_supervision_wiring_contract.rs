const RUNTIME: &str = include_str!("../src/discord_runtime.rs");
const TYPED_INGRESS: &str = include_str!("../src/discord_runtime/typed_ingress.rs");
const TYPED_SPAWN: &str = include_str!("../src/discord_runtime/typed_ingress/spawn.rs");
const GATEWAY: &str = include_str!("../../cdr-discord/src/gateway.rs");
const GATEWAY_ACTIVATION: &str = include_str!("../../cdr-discord/src/gateway/activation.rs");
const GATEWAY_SHARD: &str = include_str!("../../cdr-discord/src/gateway/shard.rs");
const SHUTDOWN_DEADLINE: &str = include_str!("../src/discord_runtime/shutdown_deadline.rs");

fn position(source: &str, needle: &str) -> usize {
    source
        .find(needle)
        .unwrap_or_else(|| panic!("missing worker-supervision marker: {needle}"))
}

#[test]
fn wsw_01_every_active_worker_can_trigger_runtime_shutdown() {
    assert!(RUNTIME.contains("mod worker_supervision;"));
    assert!(RUNTIME.contains("worker_exits.recv()"));
    assert!(RUNTIME.contains("gateway.wait_for_shard_exit()"));
    assert!(RUNTIME.contains("ShutdownCause::Worker"));
    assert!(RUNTIME.contains("ShutdownCause::GatewayShard"));
    assert!(TYPED_INGRESS.contains("WorkerExitNotifier"));
    assert!(TYPED_INGRESS.contains("spawn_monitored"));
    assert!(TYPED_SPAWN.contains("exit_notifier.clone()"));
    assert!(GATEWAY.contains("wait_for_shard_exit"));
    assert!(GATEWAY_ACTIVATION.contains("shard_exit_sender.clone()"));
    assert!(GATEWAY_SHARD.contains("ShardExitGuard"));
}

#[test]
fn wsw_02_one_absolute_deadline_bounds_drain_abort_and_join() {
    let deadline = position(RUNTIME, "let shutdown_deadline =");
    let non_heartbeat = position(RUNTIME, "let non_heartbeat_deadline =");
    let close_deadline = position(RUNTIME, "let server_close_deadline =");
    let gateway_shutdown = position(RUNTIME, "gateway.shutdown_for_cause(non_heartbeat_deadline");
    let worker_shutdown = position(RUNTIME, "workers.shutdown_for_cause(");
    let bounded_close = position(RUNTIME, "complete_before_shutdown_deadline(");
    let heartbeat_shutdown = position(RUNTIME, "heartbeat.shutdown_until(shutdown_deadline)");

    assert_eq!(RUNTIME.matches("let shutdown_deadline =").count(), 1);
    assert!(deadline < non_heartbeat);
    assert!(non_heartbeat < close_deadline);
    assert!(deadline < gateway_shutdown);
    assert!(deadline < worker_shutdown);
    assert!(worker_shutdown < bounded_close);
    assert!(bounded_close < heartbeat_shutdown);
    assert!(deadline < heartbeat_shutdown);
    assert!(RUNTIME.contains("tokio::join!("));
    assert!(SHUTDOWN_DEADLINE.contains("terminate_on_shutdown_timeout(component)"));
}

#[test]
fn wsw_03_heartbeat_signal_and_join_are_last() {
    let main_signal = position(RUNTIME, "completion_shutdown.send(true)");
    let worker_shutdown = position(RUNTIME, "workers.shutdown_for_cause(");
    let close = position(RUNTIME, "complete_before_shutdown_deadline(");
    let heartbeat_signal = position(RUNTIME, "heartbeat_shutdown.send(true)");
    let heartbeat_join = position(RUNTIME, "heartbeat.shutdown_until(shutdown_deadline)");

    assert!(main_signal < worker_shutdown);
    assert!(worker_shutdown < close);
    assert!(close < heartbeat_signal);
    assert!(heartbeat_signal < heartbeat_join);
}

#[test]
fn wsw_04_all_fallible_remote_initialization_precedes_worker_and_gateway_activation() {
    const WORKERS: &str = include_str!("../src/discord_runtime/workers.rs");
    let remote_prepare = position(RUNTIME, "prepare_remote_worker(");
    let remote_ready = remote_prepare + position(&RUNTIME[remote_prepare..], ".await?;");
    let typed_start = position(RUNTIME, "TypedIngressWorkers::start(");
    let typed_ready = typed_start + position(&RUNTIME[typed_start..], ".await?;");
    let active_wait = position(RUNTIME, "let shutdown_cause = tokio::select!");

    assert!(WORKERS.contains("ManagedRemoteAgent::initialize"));
    assert!(remote_ready < typed_start);
    assert!(
        !RUNTIME[(typed_ready + ".await?;".len())..active_wait].contains(".await?"),
        "no fallible await may delay supervision after activation"
    );
}
