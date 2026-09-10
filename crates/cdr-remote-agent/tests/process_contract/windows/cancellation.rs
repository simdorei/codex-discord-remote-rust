use std::path::Path;
use std::time::Duration;

use cdr_remote_agent::commands::{
    ProcessCompletion, run_bounded_process_cancellable, safe_environment,
};
use tempfile::TempDir;
use tokio::sync::watch;

use super::cleanup::PidCleanup;

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
                "$p=Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 30' -PassThru; [Console]::Out.Write($p.Id); [Console]::Out.Flush(); [Console]::Error.Write('cancel-ready'); [Console]::Error.Flush(); Set-Content -LiteralPath $env:CDR_PROCESS_CONTRACT_PID_PATH -Value $p.Id -NoNewline; Start-Sleep -Seconds 30".into(),
            ],
            Path::new("."),
            &environment,
            Duration::from_secs(30),
            128,
            cancelled,
        )
        .await
        .expect("cancellation is an outcome")
    });
    let Some(descendant_id) = cleanup.wait_ready(Duration::from_secs(5)).await else {
        task.abort();
        let _ = task.await;
        panic!("descendant did not publish its PID before cancellation");
    };
    cancel.send(true).expect("cancellation receiver");
    let Ok(joined) = tokio::time::timeout(Duration::from_secs(8), &mut task).await else {
        task.abort();
        let _ = task.await;
        panic!("cancellation cleanup exceeded its deadline");
    };
    let outcome = joined.expect("process task");

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
