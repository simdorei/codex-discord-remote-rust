use std::fmt::Write as _;
use std::io::Write as _;
use std::os::windows::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use cdr_remote_agent::commands::{
    ProcessCompletion, run_bounded_process, run_bounded_process_cancellable, safe_environment,
};
use tempfile::TempDir;
use tokio::sync::watch;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[path = "windows/cancellation.rs"]
mod cancellation;
#[path = "windows/cleanup.rs"]
mod cleanup;

use cleanup::{PidCleanup, settle_failure};

#[tokio::test]
async fn proc4_parent_exit_closes_owned_job_before_joining_pipe_readers() {
    let temp = TempDir::new().expect("temporary process tree directory");
    let pid_path = temp.path().join("descendant.pid");
    let mut cleanup = PidCleanup::new(pid_path.clone());
    let script_path = temp.path().join("launch-descendant.cmd");
    let test_binary = std::env::current_exe().expect("current test executable");
    std::fs::write(
        &script_path,
        format!(
            "@echo off\r\n\
             start \"\" /b \"{}\" --ignored --exact windows::pipe_holding_descendant_fixture --nocapture\r\n\
             powershell.exe -NoProfile -NonInteractive -Command \"$d=[DateTime]::UtcNow.AddSeconds(5); while (![IO.File]::Exists($env:CDR_PROCESS_CONTRACT_PID_PATH)) {{ if ([DateTime]::UtcNow -ge $d) {{ exit 7 }}; [Threading.Thread]::Sleep(10) }}\"\r\n\
             if errorlevel 1 exit /b 7\r\n\
             <nul set /p \"=parent-done\"\r\n\
             exit /b 0\r\n",
            test_binary.display(),
        ),
    )
    .expect("write process-tree launcher");
    let mut environment = safe_environment();
    environment.insert(
        "CDR_PROCESS_CONTRACT_PID_PATH".into(),
        pid_path.to_string_lossy().into_owned(),
    );

    let (cancel, cancelled) = watch::channel(false);
    let mut task = tokio::spawn(async move {
        run_bounded_process_cancellable(
            &[
                "cmd.exe".into(),
                "/D".into(),
                "/S".into(),
                "/C".into(),
                script_path.to_string_lossy().into_owned(),
            ],
            Path::new("."),
            &environment,
            Duration::from_secs(15),
            256,
            cancelled,
        )
        .await
    });
    let completed = tokio::time::timeout(Duration::from_secs(8), &mut task).await;
    let outcome = if let Ok(result) = completed {
        result.expect("owned task").expect("owned process outcome")
    } else {
        let snapshot = cleanup.diagnostic();
        let settlement = settle_failure(&mut task, &cancel).await;
        panic!("normal parent exit exceeded its original deadline; {snapshot}; {settlement}");
    };
    let descendant_id = cleanup.observe().unwrap_or_else(|| {
        panic!(
            "descendant did not publish its pid; stdout={:?}, stderr={:?}",
            String::from_utf8_lossy(&outcome.stdout),
            String::from_utf8_lossy(&outcome.stderr)
        )
    });
    assert_eq!(outcome.completion, ProcessCompletion::Exited);
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "stdout={:?}, stderr={:?}",
        String::from_utf8_lossy(&outcome.stdout),
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert!(
        outcome
            .stdout
            .windows(b"child-open".len())
            .any(|part| part == b"child-open")
    );
    assert!(outcome.stdout.ends_with(b"parent-done"));
    assert!(
        cleanup
            .wait_gone(Duration::from_secs(3))
            .expect("inspect normal-exit descendant"),
        "descendant {descendant_id} survived normal exit"
    );
}

#[test]
#[ignore = "process-tree fixture launched only by proc4"]
fn pipe_holding_descendant_fixture() {
    let Ok(pid_path) = std::env::var("CDR_PROCESS_CONTRACT_PID_PATH") else {
        return;
    };
    std::io::stdout()
        .write_all(b"child-open")
        .expect("fixture stdout");
    std::io::stdout().flush().expect("flush fixture stdout");
    std::fs::write(pid_path, std::process::id().to_string()).expect("fixture pid file");
    std::thread::sleep(Duration::from_secs(30));
}

#[tokio::test]
async fn proc5_timeout_terminates_only_the_owned_job() {
    let (_fixture, command) = timeout_fixture("owned", "");
    let mut sentinel = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("unrelated sentinel");
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        run_bounded_process(
            &command,
            Path::new("."),
            &safe_environment(),
            Duration::from_millis(750),
            128,
        ),
    )
    .await
    .expect("owned timeout cleanup deadline");
    let sentinel_running = sentinel.try_wait().expect("inspect sentinel").is_none();
    stop_child(&mut sentinel);

    let outcome = result.expect("timeout is an outcome");
    assert_eq!(outcome.completion, ProcessCompletion::TimedOut);
    assert_eq!(outcome.stdout, b"owned");
    assert!(sentinel_running, "cleanup terminated an unrelated process");
}

// Emit the fixed fixture markers before starting PowerShell. Its cold startup
// can exceed the 750 ms timeout on CI; that is unrelated to output retention.
pub(super) fn timeout_fixture(stdout: &str, stderr: &str) -> (TempDir, Vec<String>) {
    let temp = TempDir::new().expect("temporary timeout fixture");
    let path = temp.path().join("timeout.cmd");
    let mut script = format!("@echo off\r\n<nul set /p \"={stdout}\"\r\n");
    if !stderr.is_empty() {
        write!(script, "<nul set /p \"={stderr}\" 1>&2\r\n").expect("write stderr marker");
    }
    script.push_str(
        "powershell.exe -NoProfile -NonInteractive -Command \"[Threading.Thread]::Sleep(30000)\"\r\n",
    );
    std::fs::write(&path, script).expect("write timeout fixture");
    (
        temp,
        vec![
            "cmd.exe".into(),
            "/D".into(),
            "/S".into(),
            "/C".into(),
            path.to_string_lossy().into_owned(),
        ],
    )
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
