#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use std::process::{Command, Stdio};

#[test]
fn windows_powershell_defaults_repo_root_to_the_wrapper_directory() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let expected_hash = wrapper::sha256(&fixture.debug_harness);

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&fixture.script)
        .arg("-OutputDirectory")
        .arg(&fixture.evidence)
        .arg("-HarnessPath")
        .arg(&fixture.debug_harness)
        .args(["-ExpectedHarnessSha256", &expected_hash])
        .args([
            "-SkipBuild",
            "-DurationSeconds",
            "1",
            "-SampleIntervalSeconds",
            "0.1",
            "-WarmupSeconds",
            "0",
            "-HarnessExitGraceSeconds",
            "5",
            "-MaxSlopeBytesPerHour",
            "1000000000000000",
        ])
        .env("CARGO_TARGET_DIR", &fixture.target)
        .env_remove("DISCORD_BOT_TOKEN")
        .env_remove("DISCORD_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "wrapper failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let summary = wrapper::summary(&fixture);
    assert_eq!(summary["operational_status"], "passed");
    assert_eq!(summary["disabled_marker"]["preserved"], true);
    wrapper::assert_marker(&fixture);
}
