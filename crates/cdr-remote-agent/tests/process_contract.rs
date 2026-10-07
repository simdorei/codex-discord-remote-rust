use std::path::Path;
use std::time::Duration;

use cdr_remote_agent::commands::{
    ProcessCompletion, TRUNCATION_MARKER, run_bounded_process, safe_environment,
};

#[cfg(windows)]
#[path = "process_contract/windows.rs"]
mod windows;

fn fixture_command(windows_script: &str, unix_script: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            "powershell.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            windows_script.into(),
        ]
    } else {
        vec!["/bin/sh".into(), "-c".into(), unix_script.into()]
    }
}

#[cfg(windows)]
#[test]
fn proc0_windows_environment_keeps_rooted_system_directories() {
    let environment = safe_environment();
    let system_drive = environment
        .get("SystemDrive")
        .expect("SystemDrive must be preserved for Windows child processes");
    assert!(
        system_drive.len() == 2 && system_drive.ends_with(':'),
        "SystemDrive must be a drive designator, got {system_drive:?}"
    );
    for name in ["ProgramData", "SystemRoot", "TEMP", "TMP"] {
        let value = environment
            .get(name)
            .unwrap_or_else(|| panic!("{name} must be preserved for Windows child processes"));
        assert!(Path::new(value).is_absolute(), "{name} must be absolute");
        assert!(
            !value.contains('%'),
            "{name} must not contain unresolved environment references"
        );
    }
}

#[tokio::test]
async fn proc1_preserves_small_output_and_bounds_noisy_streams() {
    let environment = safe_environment();
    let small = run_bounded_process(
        &fixture_command("[Console]::Out.Write('hello')", "printf %s hello"),
        Path::new("."),
        &environment,
        Duration::from_secs(5),
        64,
    )
    .await
    .expect("small process");
    assert_eq!(small.exit_code, Some(0));
    assert_eq!(small.stdout, b"hello");
    assert_eq!(small.stdout_bytes, 5);
    assert_eq!(small.stderr_bytes, 0);
    assert!(!small.stdout_truncated);

    let nonzero = run_bounded_process(
        &fixture_command(
            "[Console]::Out.Write('out'); [Console]::Error.Write('error'); exit 7",
            "printf %s out; printf %s error >&2; exit 7",
        ),
        Path::new("."),
        &environment,
        Duration::from_secs(5),
        64,
    )
    .await
    .expect("nonzero process");
    assert_eq!(nonzero.completion, ProcessCompletion::Exited);
    assert_eq!(nonzero.exit_code, Some(7));
    assert_eq!(nonzero.stdout, b"out");
    assert_eq!(nonzero.stderr, b"error");
    assert_eq!(nonzero.stdout_bytes, 3);
    assert_eq!(nonzero.stderr_bytes, 5);

    let noisy = run_bounded_process(
        &fixture_command(
            "[Console]::Out.Write(('a' * 200)); [Console]::Error.Write(('b' * 200))",
            "i=0; while [ \"$i\" -lt 200 ]; do printf a; printf b >&2; i=$((i+1)); done",
        ),
        Path::new("."),
        &environment,
        Duration::from_secs(5),
        80,
    )
    .await
    .expect("noisy process");
    assert_eq!(noisy.stdout.len(), 80);
    assert_eq!(noisy.stderr.len(), 80);
    assert_eq!(noisy.stdout_bytes, 200);
    assert_eq!(noisy.stderr_bytes, 200);
    assert!(
        noisy
            .stdout
            .windows(TRUNCATION_MARKER.len())
            .any(|part| part == TRUNCATION_MARKER)
    );
    assert!(noisy.stdout_truncated && noisy.stderr_truncated);
}

#[tokio::test]
async fn proc2_timeout_kills_the_owned_process_tree_and_keeps_diagnostics() {
    #[cfg(windows)]
    let (_fixture, command) = windows::timeout_fixture("started", "timeout-error");
    #[cfg(not(windows))]
    let command = fixture_command(
        "",
        "printf %s started; printf %s timeout-error >&2; exec sleep 30",
    );
    let outcome = run_bounded_process(
        &command,
        Path::new("."),
        &safe_environment(),
        Duration::from_millis(750),
        128,
    )
    .await
    .expect("timeout is an outcome");
    assert_eq!(outcome.completion, ProcessCompletion::TimedOut);
    assert_eq!(outcome.exit_code, None);
    assert_eq!(outcome.stdout, b"started");
    assert_eq!(outcome.stderr, b"timeout-error");
    assert_eq!(outcome.stdout_bytes, 7);
    assert_eq!(outcome.stderr_bytes, 13);
}
