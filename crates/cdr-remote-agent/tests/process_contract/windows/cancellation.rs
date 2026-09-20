use std::path::Path;
use std::time::Duration;

use cdr_remote_agent::commands::{
    ProcessCompletion, run_bounded_process_cancellable, safe_environment,
};
use tempfile::TempDir;
use tokio::sync::watch;

use super::cleanup::{PidCleanup, settle_failure};

#[tokio::test]
async fn proc3_bridge_cancellation_kills_the_owned_process_tree() {
    let temp = TempDir::new().expect("temporary cancellation directory");
    let pid_path = temp.path().join("descendant.pid");
    let mut cleanup = PidCleanup::new(pid_path.clone());
    let mut environment = safe_environment();
    environment.insert(
        "CDR_PROCESS_CONTRACT_PID_PATH".into(),
        pid_path.to_string_lossy().into_owned(),
    );
    let (cancel, cancelled) = watch::channel(false);
    let mut task = tokio::spawn(async move {
        run_bounded_process_cancellable(
            &[
                "powershell.exe".into(),
                "-NoProfile".into(),
                "-Command".into(),
                // Avoid the Management cmdlets: they can stall fixture setup
                // on the hosted runner before the cancellation contract begins.
                r#"$ErrorActionPreference = 'Stop'
$start = [Diagnostics.ProcessStartInfo]::new('powershell.exe')
$start.Arguments = '-NoProfile -NonInteractive -Command "[Threading.Thread]::Sleep(30000)"'
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$child = [Diagnostics.Process]::Start($start)
[Console]::Out.Write($child.Id)
[Console]::Out.Flush()
[Console]::Error.Write('cancel-ready')
[Console]::Error.Flush()
[IO.File]::WriteAllText($env:CDR_PROCESS_CONTRACT_PID_PATH, [string]$child.Id)
[Threading.Thread]::Sleep(30000)
"#
                .into(),
            ],
            Path::new("."),
            &environment,
            Duration::from_secs(30),
            128,
            cancelled,
        )
        .await
    });
    let ready = tokio::select! {
        ready = cleanup.wait_ready(Duration::from_secs(5)) => ready,
        result = &mut task => panic!("operation ended before PID readiness: {result:?}; {}", cleanup.diagnostic()),
    };
    let Some(descendant_id) = ready else {
        let snapshot = cleanup.diagnostic();
        let settlement = settle_failure(&mut task, &cancel).await;
        panic!(
            "descendant did not publish a parseable PID before cancellation; {snapshot}; {settlement}"
        );
    };
    cancel.send(true).expect("cancellation receiver");
    let Ok(joined) = tokio::time::timeout(Duration::from_secs(8), &mut task).await else {
        let snapshot = cleanup.diagnostic();
        let settlement = settle_failure(&mut task, &cancel).await;
        panic!("cancellation cleanup exceeded its deadline; {snapshot}; {settlement}");
    };
    let outcome = joined
        .expect("process task")
        .expect("cancellation is an outcome");

    assert_eq!(outcome.completion, ProcessCompletion::Cancelled);
    assert_eq!(outcome.exit_code, None);
    assert_eq!(outcome.stdout, descendant_id.to_string().as_bytes());
    assert_eq!(outcome.stderr, b"cancel-ready");
    assert_eq!(outcome.stdout_bytes, outcome.stdout.len() as u64);
    assert_eq!(outcome.stderr_bytes, 12);
    assert!(
        cleanup
            .wait_gone(Duration::from_secs(3))
            .expect("inspect cancelled descendant"),
        "descendant survived cancellation"
    );
}
