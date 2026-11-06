use super::RuntimePaths;
use cdr_app_server::{AppServerError, ResidentAppServer};
use serde_json::json;
use std::io::Write;

pub async fn write(paths: &RuntimePaths, server: &ResidentAppServer) -> Result<(), AppServerError> {
    let snapshot = server.lifecycle_snapshot().await;
    let pid = snapshot
        .process_id
        .ok_or_else(|| AppServerError::StartupHeld("initialized process PID unavailable".into()))?;
    let process = cdr_windows_native::query_live_process_metadata(pid)?.ok_or_else(|| {
        AppServerError::StartupHeld("initialized process identity unavailable".into())
    })?;
    if !snapshot.healthy
        || std::fs::canonicalize(&process.executable_path)?
            != std::fs::canonicalize(&paths.codex_exe)?
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
    let mut temporary = tempfile::NamedTempFile::new_in(&paths.root)?;
    serde_json::to_writer(&mut temporary, &record)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    cdr_windows_native::atomic_replace(
        temporary.path(),
        &paths.root.join(".codex_discord_rust.codex-selection.json"),
    )?;
    eprintln!(
        "codex_selection_initialized: {}",
        serde_json::to_string(&record)?
    );
    Ok(())
}
