#![cfg(windows)]

use std::collections::HashMap;
use std::io::Read as _;
use std::path::Path;
use std::time::{Duration, Instant};

use cdr_windows_native::CapturedWindowProcess;

#[test]
fn native_process_captures_streams_and_preserves_environment_and_cwd() {
    let temp = tempfile::tempdir().expect("temporary working directory");
    let mut environment = std::env::vars().collect::<HashMap<_, _>>();
    environment.insert("CDR_PROCESS_VALUE".into(), "two words".into());
    let mut process = CapturedWindowProcess::launch(
        Path::new("powershell.exe"),
        &[
            "-NoProfile".into(),
            "-Command".into(),
            "[Console]::Out.Write($env:CDR_PROCESS_VALUE + '|' + [Environment]::CurrentDirectory); [Console]::Error.Write('problem')".into(),
        ],
        temp.path(),
        &environment,
    )
    .expect("captured process launch");
    let process_id = process.process_id();
    let (exit_code, output, error) = collect_output(&mut process);

    assert!(process_id > 0);
    assert_eq!(exit_code, 0);
    assert_eq!(
        String::from_utf8(output).expect("UTF-8 stdout"),
        format!("two words|{}", temp.path().display())
    );
    assert_eq!(error, b"problem");
}

#[test]
fn native_process_resolves_bare_program_from_the_child_path() {
    let temp = tempfile::tempdir().expect("temporary process directory");
    let bin = temp.path().join("child-bin");
    std::fs::create_dir(&bin).expect("child bin directory");
    let probe = bin.join("cdr-child-path-probe.exe");
    std::fs::copy(
        std::env::current_exe().expect("current test executable"),
        &probe,
    )
    .expect("copy process probe");
    let mut environment = std::env::vars().collect::<HashMap<_, _>>();
    environment.insert("PATH".into(), bin.to_string_lossy().into_owned());
    environment.insert(
        "path".into(),
        temp.path().join("decoy").display().to_string(),
    );
    let mut process = CapturedWindowProcess::launch(
        Path::new("cdr-child-path-probe"),
        &[
            "--ignored".into(),
            "--exact".into(),
            "child_path_fixture".into(),
            "--nocapture".into(),
        ],
        temp.path(),
        &environment,
    )
    .expect("bare child-PATH process launch");
    let (exit_code, output, error) = collect_output(&mut process);

    assert_eq!(exit_code, 0);
    assert!(
        output
            .windows(b"child-path-resolved".len())
            .any(|part| part == b"child-path-resolved"),
        "stdout={:?}",
        String::from_utf8_lossy(&output)
    );
    assert!(error.is_empty());
}

#[test]
#[ignore = "child process fixture launched by the child-PATH contract"]
fn child_path_fixture() {
    print!("child-path-resolved");
}

fn collect_output(process: &mut CapturedWindowProcess) -> (i32, Vec<u8>, Vec<u8>) {
    let mut stdout = process.take_stdout().expect("stdout pipe");
    let mut stderr = process.take_stderr().expect("stderr pipe");
    let deadline = Instant::now() + Duration::from_secs(5);
    let exit_code = loop {
        if let Some(exit_code) = process.try_wait().expect("process wait") {
            break exit_code;
        }
        assert!(Instant::now() < deadline, "process did not exit");
        std::thread::sleep(Duration::from_millis(10));
    };
    process
        .terminate_tree(Duration::from_secs(2))
        .expect("close owned job");
    let mut output = Vec::new();
    let mut error = Vec::new();
    stdout.read_to_end(&mut output).expect("read stdout");
    stderr.read_to_end(&mut error).expect("read stderr");
    (exit_code, output, error)
}
