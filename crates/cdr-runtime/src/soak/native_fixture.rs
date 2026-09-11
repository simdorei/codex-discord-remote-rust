//! Start the compiled offline harness, never an interpreter or the real Codex app.
use cdr_app_server::AppServerConfig;

#[must_use]
pub fn config(scenario: &str) -> AppServerConfig {
    let test = std::env::current_exe().unwrap();
    let artifacts = test.parent().unwrap().parent().unwrap();
    let executable = artifacts.join(format!("cdr-offline-soak{}", std::env::consts::EXE_SUFFIX));
    assert!(
        executable.is_file(),
        "native fixture missing: {}; build with cargo build -p cdr-runtime --bin cdr-offline-soak before a --lib-only test run",
        executable.display()
    );
    let mut config = AppServerConfig::new(executable);
    config.arguments = vec!["--app-server-fixture".into(), scenario.into()];
    config
}
