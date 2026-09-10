use std::fs;

#[test]
fn discord_runtime_prepares_remote_handoff_before_signalling_shutdown() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/discord_runtime.rs"),
    )
    .unwrap();
    let workers = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/discord_runtime/workers.rs"),
    )
    .unwrap();

    assert!(workers.contains("RestartHandoffRuntime::system"));
    assert!(workers.contains("ManagedRemoteAgent::initialize"));
    let start = source.find("let (remote_worker, remote_control)").unwrap();
    let prepare = source
        .find("request_remote_handoff_if_needed(&paths")
        .unwrap();
    let shutdown = source.find("completion_shutdown.send(true)").unwrap();
    assert!(start < prepare);
    assert!(prepare < shutdown);
}

#[test]
fn runtime_switches_are_applied_to_their_workers() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/discord_runtime.rs"),
    )
    .unwrap();

    assert!(source.contains("config.stream_commentary"));
    assert!(source.contains("start_session_mirror_worker(\n        config.session_mirror"));
}
