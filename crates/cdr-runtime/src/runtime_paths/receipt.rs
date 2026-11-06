use super::RuntimePaths;
use cdr_app_server::{AppServerError, ResidentAppServer};
use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Finish the initialized startup transaction, retaining time to reap the server on failure.
pub async fn write(
    paths: &RuntimePaths,
    server: &Arc<ResidentAppServer>,
    deadline: Instant,
) -> Result<(), AppServerError> {
    match std::env::current_exe() {
        Ok(runtime) => write_using(&runtime, paths, server, deadline).await,
        Err(error) => Err(close_on_error(server, error.into(), deadline).await),
    }
}

pub async fn write_using(
    runtime: &Path,
    paths: &RuntimePaths,
    server: &Arc<ResidentAppServer>,
    deadline: Instant,
) -> Result<(), AppServerError> {
    match write_inner(runtime, paths, server, deadline).await {
        Ok(()) => Ok(()),
        Err(error) => Err(close_on_error(server, error, deadline).await),
    }
}

async fn close_on_error(
    server: &Arc<ResidentAppServer>,
    primary: AppServerError,
    deadline: Instant,
) -> AppServerError {
    let owned = Arc::clone(server);
    // A timed-out waiter must not cancel the process owner's cleanup.
    let cleanup = tokio::spawn(async move { owned.close().await });
    let result =
        match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), cleanup).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(AppServerError::StartupHeld(format!(
                "cleanup_unconfirmed: {error}"
            ))),
            Err(_) => Err(AppServerError::StartupHeld(
                "cleanup_unconfirmed: receipt startup deadline".into(),
            )),
        };
    match result {
        Ok(()) => primary,
        Err(cleanup) => AppServerError::StartupCleanup {
            primary: Box::new(primary),
            cleanup: Box::new(cleanup),
        },
    }
}

async fn write_inner(
    runtime: &Path,
    paths: &RuntimePaths,
    server: &ResidentAppServer,
    deadline: Instant,
) -> Result<(), AppServerError> {
    let snapshot = server.lifecycle_snapshot().await;
    let pid = snapshot
        .process_id
        .ok_or_else(|| AppServerError::StartupHeld("initialized process PID unavailable".into()))?;
    let process = cdr_windows_native::query_live_process_metadata(pid)?.ok_or_else(|| {
        AppServerError::StartupHeld("initialized process identity unavailable".into())
    })?;
    if !snapshot.healthy
        || !cdr_windows_native::same_resolved_path(&process.executable_path, &paths.codex_exe)
    {
        return Err(AppServerError::StartupHeld(
            "initialized process does not match selection".into(),
        ));
    }
    let record = json!({
        "schema_version":1,
        "observed_at":chrono::Utc::now().to_rfc3339(),
        "runtime_process_identity":cdr_windows_native::current_process_identity()?,
        "app_server_process_identity":process.process_identity,
        "app_server_pid":pid,
        "generation":snapshot.generation,
        "codex_home":paths.codex_home,
        "selection":paths.codex_selection,
        "initialize_completed":true,
        "tools":{"contract_id":"local-tools-v1","state":"NOT_CHECKED"}
    });
    let record = serde_json::to_string(&record)?;
    let worker_deadline = deadline
        .checked_sub(Duration::from_secs(8))
        .ok_or_else(|| AppServerError::StartupHeld("receipt deadline exhausted".into()))?;
    // No parent-side temporary/request file: all receipt filesystem I/O is owned by this Job.
    let output = cdr_windows_native::capture_bounded(
        runtime,
        &[
            "--admin".into(),
            "codex-selection-worker".into(),
            "--record-json".into(),
            record.clone(),
            "--repo-root".into(),
            paths.root.display().to_string(),
        ],
        &paths.root,
        &std::env::vars().collect(),
        worker_deadline,
        Duration::from_secs(1),
        65_536,
    )?;
    if output.code != 0
        || output.stdout != b"receipt_written\n" && output.stdout != b"receipt_written\r\n"
    {
        return Err(AppServerError::StartupHeld(format!(
            "receipt write unconfirmed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    eprintln!("codex_selection_initialized: {record}");
    Ok(())
}

/// Called only by the owned receipt worker; every filesystem operation stays in that process.
pub fn persist(root: &Path, record: &str) -> Result<String, AppServerError> {
    let record: serde_json::Value = serde_json::from_str(record)?;
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer(&mut temporary, &record)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    cdr_windows_native::atomic_replace(
        temporary.path(),
        &root.join(".codex_discord_rust.codex-selection.json"),
    )?;
    Ok("receipt_written".into())
}
