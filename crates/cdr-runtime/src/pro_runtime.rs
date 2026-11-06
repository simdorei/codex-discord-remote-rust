mod capture;
mod diagnostics;
mod project;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cdr_app_server::ResidentAppServer;
use cdr_pro::diagnostics::{DiagnosticCode, ProError};
use cdr_pro::preflight::{ResidentSnapshot, expected_remote_plugin_version, verify_runtime};
use cdr_pro::prompt::{DeviceTicket, format_local_device_prompt, rewrite_pro_prompt};
use cdr_remote_agent::config::{RemoteMcpConfig, RemoteMcpOwnership};
use cdr_remote_agent::status::{
    EXTERNAL_MCP_STATUS_SCHEMA_VERSION, ExternalMcpStatusRecord, RemoteAgentStatus,
    normalize_install_root, sha256_file_hex,
};
use chrono::Utc;
use tokio::sync::Mutex;

use crate::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessError, PromptPreprocessor};
use capture::{CapturedPlugins, capture_plugins};
use diagnostics::{
    connection_failure, external_connection_failure, not_configured, public_error, restart_failure,
};

struct ResidentPluginBaseline {
    generation: u64,
    fingerprint: Result<String, String>,
}

#[derive(Default)]
struct ExternalMcpEpochTracker {
    current: Option<(String, u64)>,
    retired_identities: HashSet<String>,
}

pub struct ProPromptRuntime {
    root: PathBuf,
    external_status_path: PathBuf,
    state_db: PathBuf,
    codex_exe: PathBuf,
    manifest: PathBuf,
    server: Arc<ResidentAppServer>,
    remote: Option<RemoteMcpConfig>,
    remote_status: RemoteAgentStatus,
    baseline: Mutex<ResidentPluginBaseline>,
    external_epoch: Mutex<ExternalMcpEpochTracker>,
}

fn is_consistent_committed_ownership_json(json: &serde_json::Value, expected_mode: &str) -> bool {
    let schema_ok = json
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        == Some(1);
    let mode = json
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let state_field = json.get("state").and_then(serde_json::Value::as_str);
    let committed_field = json.get("committed").and_then(serde_json::Value::as_bool);
    let state_committed = match (state_field, committed_field) {
        (Some(s), Some(b)) => s.eq_ignore_ascii_case("committed") && b,
        (Some(s), None) => s.eq_ignore_ascii_case("committed"),
        (None, Some(b)) => b,
        (None, None) => false,
    };
    schema_ok && mode.eq_ignore_ascii_case(expected_mode) && state_committed
}

#[must_use]
pub fn effective_remote_mcp_ownership(
    root: &Path,
    configured: RemoteMcpOwnership,
) -> RemoteMcpOwnership {
    if configured == RemoteMcpOwnership::External {
        return RemoteMcpOwnership::External;
    }
    let state_path = root
        .join(".cdr-maintenance")
        .join("mcp-ownership-state.json");
    let bytes = match fs::read(&state_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return RemoteMcpOwnership::Embedded;
        }
        // Fail closed against embedded startup when ownership state exists but is unreadable.
        Err(_) => return RemoteMcpOwnership::External,
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        // Malformed migration state must never revive an embedded connector alongside external MCP.
        return RemoteMcpOwnership::External;
    };
    if is_consistent_committed_ownership_json(&json, "embedded") {
        RemoteMcpOwnership::Embedded
    } else {
        RemoteMcpOwnership::External
    }
}

pub fn verify_external_ownership_state_committed(
    root: &Path,
    configured: RemoteMcpOwnership,
) -> Result<(), ProError> {
    let state_path = root
        .join(".cdr-maintenance")
        .join("mcp-ownership-state.json");
    let bytes = match fs::read(&state_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if configured == RemoteMcpOwnership::External {
                return Ok(());
            }
            return Err(external_connection_failure(
                "external remote MCP ownership state file is missing",
            ));
        }
        Err(_) => {
            return Err(external_connection_failure(
                "external remote MCP ownership state file is unreadable",
            ));
        }
    };
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        external_connection_failure("external remote MCP ownership state file is invalid JSON")
    })?;
    if !is_consistent_committed_ownership_json(&json, "external") {
        return Err(external_connection_failure(
            "external remote MCP ownership transition is not committed",
        ));
    }
    Ok(())
}

impl ProPromptRuntime {
    pub async fn capture(
        root: PathBuf,
        state_db: PathBuf,
        codex_exe: PathBuf,
        server: Arc<ResidentAppServer>,
        remote: Option<RemoteMcpConfig>,
        remote_status: RemoteAgentStatus,
    ) -> Self {
        let generation = server.generation();
        let fingerprint = capture_plugins(&codex_exe)
            .await
            .map(|capture| capture.fingerprint)
            .map_err(|diagnostic| diagnostic.internal_detail);
        let manifest = root.join("plugins/codex-discord-remote/.codex-plugin/plugin.json");
        let external_status_path = remote.as_ref().map_or_else(
            || root.join("bot_data").join("remote_mcp_status.json"),
            |cfg| cfg.resolve_status_file_path(&root),
        );
        Self {
            root,
            external_status_path,
            state_db,
            codex_exe,
            manifest,
            server,
            remote,
            remote_status,
            baseline: Mutex::new(ResidentPluginBaseline {
                generation,
                fingerprint,
            }),
            external_epoch: Mutex::new(ExternalMcpEpochTracker::default()),
        }
    }

    #[must_use]
    pub fn with_external_status_path(mut self, path: PathBuf) -> Self {
        self.external_status_path = path;
        self
    }

    pub async fn current_connection_epoch(&self) -> Result<(String, u64), PromptPreprocessError> {
        let Some(config) = &self.remote else {
            return Err(public_error(&not_configured()));
        };
        if effective_remote_mcp_ownership(&self.root, config.ownership)
            == RemoteMcpOwnership::External
        {
            self.verify_external_mcp_ready(config)
                .await
                .map_err(|error| public_error(&error))
        } else if self.remote_status.is_connected() {
            Ok((
                "embedded".to_owned(),
                self.remote_status.generation().max(1),
            ))
        } else {
            Err(public_error(&connection_failure()))
        }
    }

    pub async fn verify_epoch_unchanged(
        &self,
        expected: &(String, u64),
    ) -> Result<(), PromptPreprocessError> {
        let current = self.current_connection_epoch().await?;
        if &current != expected {
            return Err(public_error(&external_connection_failure(
                "remote MCP connection epoch changed during turn",
            )));
        }
        Ok(())
    }

    async fn prepare_pro(
        &self,
        prompt: &str,
        thread_id: &str,
    ) -> Result<String, PromptPreprocessError> {
        let Some(rewritten) = rewrite_pro_prompt(prompt) else {
            return Ok(prompt.to_owned());
        };
        self.preflight_with_recovery()
            .await
            .map_err(|error| public_error(&error))?;
        let Some(config) = &self.remote else {
            return Err(public_error(&not_configured()));
        };
        if effective_remote_mcp_ownership(&self.root, config.ownership)
            == RemoteMcpOwnership::External
        {
            self.verify_external_mcp_ready(config)
                .await
                .map_err(|error| public_error(&error))?;
        } else if !self.remote_status.is_connected() {
            return Err(public_error(&connection_failure()));
        }
        Ok(format_local_device_prompt(
            &rewritten,
            thread_id,
            &DeviceTicket {
                device_id: config.device_id.clone(),
                working_directory: project::working_directory(&self.state_db, thread_id)?,
            },
        ))
    }

    async fn verify_external_mcp_ready(
        &self,
        config: &RemoteMcpConfig,
    ) -> Result<(String, u64), ProError> {
        verify_external_ownership_state_committed(&self.root, config.ownership)?;
        let bytes = fs::read(&self.external_status_path).map_err(|_| {
            external_connection_failure("external remote MCP status file is missing or unreadable")
        })?;
        let record: ExternalMcpStatusRecord = serde_json::from_slice(&bytes).map_err(|_| {
            external_connection_failure("external remote MCP status file is invalid JSON")
        })?;
        if record.schema_version != EXTERNAL_MCP_STATUS_SCHEMA_VERSION || record.component != "mcp"
        {
            return Err(external_connection_failure(
                "external remote MCP status schema or component mismatch",
            ));
        }
        if record.install_root != normalize_install_root(&self.root) {
            return Err(external_connection_failure(
                "external remote MCP status install_root mismatch",
            ));
        }
        if record.device_id != config.device_id
            || record.bridge_fingerprint != config.bridge_fingerprint()
        {
            return Err(external_connection_failure(
                "external remote MCP device_id or bridge configuration mismatch",
            ));
        }
        let now_ms = Utc::now().timestamp_millis();
        let stale_ms = cdr_remote_agent::status::bridge_liveness_stale_ms();
        let updated_age = now_ms.saturating_sub(record.updated_at_ms);
        let heartbeat_age = now_ms.saturating_sub(record.heartbeat_at_ms);
        if record.updated_at_ms <= 0
            || record.heartbeat_at_ms <= 0
            || record.updated_at_ms > now_ms + 2_000
            || record.heartbeat_at_ms > now_ms + 2_000
            || record.updated_at_ms > record.heartbeat_at_ms + 2_000
            || updated_age > stale_ms
            || heartbeat_age > stale_ms
        {
            return Err(external_connection_failure(
                "external remote MCP status record is stale",
            ));
        }
        let Some(verified_ms) = record.connection_verified_at_ms else {
            return Err(external_connection_failure(
                "external remote MCP bridge connection liveness evidence is missing",
            ));
        };
        let verified_age = now_ms.saturating_sub(verified_ms);
        if verified_ms <= 0
            || verified_ms > now_ms + 2_000
            || verified_ms > record.heartbeat_at_ms + 2_000
            || verified_age > stale_ms
        {
            return Err(external_connection_failure(
                "external remote MCP bridge connection liveness is stale",
            ));
        }
        if !record.hello_ack || !record.ready || record.draining || record.generation == 0 {
            return Err(external_connection_failure(
                "external remote MCP is not ready or gateway hello is unacknowledged",
            ));
        }
        if let Some(parent) = self.external_status_path.parent() {
            let lock_file = parent.join("remote_mcp.instance.lock");
            if let Ok(lock_text) = fs::read_to_string(&lock_file) {
                let lock_identity = lock_text.trim();
                if !lock_identity.is_empty() && lock_identity != record.process_identity {
                    return Err(external_connection_failure(
                        "external remote MCP status process_identity does not match active instance lock",
                    ));
                }
            }
        }
        verify_live_external_mcp_process(&record)?;

        let mut tracker = self.external_epoch.lock().await;
        if tracker
            .retired_identities
            .contains(&record.process_identity)
        {
            return Err(external_connection_failure(
                "external remote MCP status references a retired process_identity",
            ));
        }
        if let Some((prev_identity, prev_gen)) = tracker.current.clone() {
            if prev_identity == record.process_identity {
                if record.generation < prev_gen {
                    return Err(external_connection_failure(
                        "external remote MCP generation regressed within the same process",
                    ));
                }
            } else {
                tracker.retired_identities.insert(prev_identity);
            }
        }
        let epoch = (record.process_identity, record.generation);
        tracker.current = Some(epoch.clone());
        Ok(epoch)
    }

    async fn preflight_with_recovery(&self) -> Result<(), ProError> {
        let error = match self.preflight_once().await {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if error.diagnostic().map(|value| value.code) != Some(DiagnosticCode::ResidentStale) {
            return Err(error);
        }
        match self.server.force_restart_if_quiescent().await {
            Ok(true) => self.preflight_once().await,
            Ok(false) => Err(error),
            Err(refresh_error) => Err(restart_failure(error, &refresh_error)),
        }
    }

    async fn preflight_once(&self) -> Result<(), ProError> {
        let current = capture_plugins(&self.codex_exe)
            .await
            .map_err(ProError::Preflight)?;
        let expected = expected_remote_plugin_version(&self.manifest)?;
        let lifecycle = self.server.lifecycle_snapshot().await;
        let resident = self
            .resident_snapshot(lifecycle.generation, lifecycle.healthy, &current)
            .await;
        verify_runtime(
            &current.inventory,
            &expected,
            &resident,
            &current.fingerprint,
        )?;
        Ok(())
    }

    async fn resident_snapshot(
        &self,
        generation: u64,
        healthy: bool,
        current: &CapturedPlugins,
    ) -> ResidentSnapshot {
        let mut baseline = self.baseline.lock().await;
        if baseline.generation != generation {
            baseline.generation = generation;
            baseline.fingerprint = Ok(current.fingerprint.clone());
        }
        ResidentSnapshot {
            generation,
            healthy,
            accepting: healthy,
            plugin_runtime_fingerprint: baseline.fingerprint.clone().ok(),
            plugin_runtime_error: baseline.fingerprint.clone().err(),
        }
    }
}

#[cfg(windows)]
fn verify_live_external_mcp_process(record: &ExternalMcpStatusRecord) -> Result<(), ProError> {
    if record.build_info.schema_version != 1 {
        return Err(external_connection_failure(
            "external remote MCP build_info schema mismatch",
        ));
    }
    let Some((pid_part, _)) = record.process_identity.split_once('|') else {
        return Err(external_connection_failure(
            "external remote MCP process_identity format is invalid",
        ));
    };
    let Ok(identity_pid) = pid_part.parse::<u32>() else {
        return Err(external_connection_failure(
            "external remote MCP process_identity pid is invalid",
        ));
    };
    if identity_pid != record.pid {
        return Err(external_connection_failure(
            "external remote MCP pid does not match process_identity",
        ));
    }
    let live = cdr_windows_native::query_live_process_metadata(record.pid)
        .map_err(|_| external_connection_failure("failed to inspect live external MCP process"))?
        .ok_or_else(|| external_connection_failure("external remote MCP process is not running"))?;
    if live.process_identity != record.process_identity {
        return Err(external_connection_failure(
            "external remote MCP process start time does not match process_identity",
        ));
    }
    if normalize_install_root(&live.executable_path)
        != normalize_install_root(Path::new(&record.executable_path))
    {
        return Err(external_connection_failure(
            "external remote MCP executable_path does not match live process image",
        ));
    }
    let live_hash = sha256_file_hex(&live.executable_path).map_err(|_| {
        external_connection_failure("failed to hash live external MCP executable artifact")
    })?;
    if !record.artifact_sha256.eq_ignore_ascii_case(&live_hash) {
        return Err(external_connection_failure(
            "external remote MCP executable artifact hash mismatch",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn verify_live_external_mcp_process(_record: &ExternalMcpStatusRecord) -> Result<(), ProError> {
    Ok(())
}

impl PromptPreprocessor for ProPromptRuntime {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move { self.prepare_pro(prompt, thread_id).await })
    }
}
