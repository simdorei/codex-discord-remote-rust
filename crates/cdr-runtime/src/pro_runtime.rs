mod capture;
mod diagnostics;
mod project;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use cdr_app_server::ResidentAppServer;
use cdr_pro::diagnostics::{DiagnosticCode, ProError};
use cdr_pro::preflight::{ResidentSnapshot, expected_remote_plugin_version, verify_runtime};
use cdr_pro::prompt::{DeviceTicket, format_local_device_prompt, rewrite_pro_prompt};
use cdr_remote_agent::config::RemoteMcpConfig;
use cdr_remote_agent::status::RemoteAgentStatus;
use tokio::sync::Mutex;

use crate::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessError, PromptPreprocessor};
use capture::{CapturedPlugins, capture_plugins};
use diagnostics::{connection_failure, not_configured, public_error, restart_failure};

struct ResidentPluginBaseline {
    generation: u64,
    fingerprint: Result<String, String>,
}

pub struct ProPromptRuntime {
    state_db: PathBuf,
    codex_exe: PathBuf,
    manifest: PathBuf,
    server: Arc<ResidentAppServer>,
    remote: Option<RemoteMcpConfig>,
    remote_status: RemoteAgentStatus,
    baseline: Mutex<ResidentPluginBaseline>,
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
        Self {
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
        }
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
        if !self.remote_status.is_connected() {
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

impl PromptPreprocessor for ProPromptRuntime {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move { self.prepare_pro(prompt, thread_id).await })
    }
}
