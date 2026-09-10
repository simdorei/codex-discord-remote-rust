const RUNTIME: &str = include_str!("../src/discord_runtime.rs");
const INTERACTION: &str = include_str!("../src/discord_runtime/typed_ingress/interaction.rs");
const SPAWN: &str = include_str!("../src/discord_runtime/typed_ingress/spawn.rs");
const GATEWAY_LOOP: &str = include_str!("../src/discord_runtime/gateway_loop.rs");

fn compact(source: &str) -> String {
    source.split_whitespace().collect()
}

#[test]
fn tic_04_two_independent_lanes_use_exact_concurrency_and_one_claim_cache() {
    let interaction = compact(INTERACTION);
    let spawn = compact(SPAWN);
    assert!(interaction.contains("constNORMAL_CONCURRENCY:usize=16;"));
    assert!(interaction.contains("constRESERVED_CONCURRENCY:usize=4;"));
    assert!(spawn.contains("\"interaction-normal\""));
    assert!(spawn.contains("\"interaction-reserved\""));
    assert_eq!(
        interaction.matches("InteractionClaimCache::new(").count(),
        1
    );
}

#[test]
fn tic_05_policy_is_snapshotted_before_activation_and_never_loaded_on_ack_path() {
    let runtime = compact(RUNTIME);
    let interaction = compact(INTERACTION);
    let snapshot = runtime
        .find("interaction_policy(")
        .expect("policy snapshot");
    let consumers = runtime
        .find("TypedIngressWorkers::start(")
        .expect("typed consumer startup");
    assert!(snapshot < consumers);
    assert!(!interaction.contains("interaction_policy("));
    assert!(!interaction.contains("mirror_db"));
    assert!(!interaction.contains("rusqlite"));
}

#[test]
fn tic_06_legacy_gateway_loop_cannot_dispatch_interactions_in_parallel() {
    let gateway = compact(GATEWAY_LOOP);
    assert!(!gateway.contains("InteractionCreate"));
    assert!(!gateway.contains("InteractionDispatcher"));
    assert!(!gateway.contains("interaction_policy("));
}

#[test]
fn tic_08_runtime_marks_ingress_stopping_before_notifying_consumers() {
    let runtime = compact(RUNTIME);
    let stopping = runtime
        .find("gateway.begin_stopping();")
        .expect("gateway ingress stopping boundary");
    let notify = runtime
        .find("completion_shutdown.send(true)")
        .expect("consumer shutdown notification");
    let gateway_shutdown = runtime
        .find("gateway.shutdown_for_cause(non_heartbeat_deadline")
        .expect("gateway shutdown");

    assert!(stopping < notify);
    assert!(notify < gateway_shutdown);
}
