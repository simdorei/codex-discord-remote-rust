use std::fs;

#[test]
fn production_runtime_wires_pro_preflight_remote_readiness_and_rich_turn_input() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let bootstrap = fs::read_to_string(root.join("src/discord_runtime/bootstrap.rs")).unwrap();
    let runtime = fs::read_to_string(root.join("src/discord_runtime.rs")).unwrap();
    let workers = fs::read_to_string(root.join("src/discord_runtime/workers.rs")).unwrap();
    let backend = fs::read_to_string(root.join("src/app_backend.rs")).unwrap();
    let pro = fs::read_to_string(root.join("src/pro_runtime.rs")).unwrap();

    assert!(bootstrap.contains("ProPromptRuntime::capture"));
    assert!(bootstrap.contains("with_prompt_preprocessor"));
    assert!(runtime.contains("start_remote_worker"));
    assert!(workers.contains("ManagedRemoteAgent::initialize"));
    assert!(pro.contains("remote_status.is_connected"));
    assert!(pro.contains("preflight_with_recovery"));
    assert!(backend.contains("build_turn_input"));
    assert!(backend.contains("start_turn_with_input"));
}
