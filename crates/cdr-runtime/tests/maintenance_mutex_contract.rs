#![cfg(windows)]
use cdr_runtime::runtime_instance::{RuntimeInstanceError, RuntimeInstanceGuard};
use std::{
    io::{BufRead, Write},
    process::{Command, Stdio},
};

#[test]
fn legacy_spelling_mutex_in_another_process_preserves_runtime_marker() {
    let temp = tempfile::tempdir().unwrap();
    // Match the installed runtime's non-verbatim spelling, not canonicalize().
    let spelling = temp
        .path()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    let root = std::path::Path::new(&spelling);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "separate_process_mutex_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("CDR_MUTEX_FIXTURE_ROOT", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert!(
            output.read_line(&mut line).unwrap() > 0,
            "fixture exited before acquiring mutex"
        );
        if line.contains("fixture_ready") {
            break;
        }
    }
    let marker = root.join(".codex_discord_rust.runtime.lock");
    let before = std::fs::read(&marker).unwrap();
    assert!(matches!(
        RuntimeInstanceGuard::acquire(root),
        Err(RuntimeInstanceError::AlreadyRunning)
    ));
    assert_eq!(before, std::fs::read(&marker).unwrap());
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    assert!(!marker.exists());
}

#[test]
#[ignore = "child process fixture, never starts a bot"]
fn separate_process_mutex_fixture() {
    let Some(root) = std::env::var_os("CDR_MUTEX_FIXTURE_ROOT") else {
        return;
    };
    let _guard = RuntimeInstanceGuard::acquire(std::path::Path::new(&root)).unwrap();
    println!("fixture_ready");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
}
