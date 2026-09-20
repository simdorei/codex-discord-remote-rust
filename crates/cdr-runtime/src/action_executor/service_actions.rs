use cdr_app_server::requests::{list_models, rate_limits};

use super::model_catalog::reserve;
use super::{ActionError, ActionExecutor, ActionResult, immediate};
use crate::queue_runner::TurnBackend;
#[cfg(test)]
#[path = "diagnostic_connected_tests.rs"]
mod connected_tests;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn doctor(&self) -> Result<ActionResult, ActionError> {
        let text = crate::diagnostic_report::report(crate::diagnostic_report::Paths {
            state: self.state_db.clone(),
            mirror: self.mirror_db.clone(),
            bridge: self.bridge_state.path().to_owned(),
        })
        .await
        .map_err(ActionError::Invalid)?;
        let lifecycle = if let Some(server) = &self.server {
            format!("{:?}", server.lifecycle_snapshot().await)
        } else {
            "미확인: app-server 연결 없음".into()
        };
        Ok(immediate(format!(
            "{text}\napp_server: {lifecycle}\nruntime_pid: {}",
            std::process::id()
        )))
    }

    pub(super) async fn settings_options(
        &self,
        channel_id: u64,
        reference: Option<&str>,
        field: Option<&str>,
    ) -> Result<ActionResult, ActionError> {
        tokio::time::timeout(
            self.app_server_resume_timeout
                .min(std::time::Duration::from_secs(8)),
            self.settings_options_inner(channel_id, reference, field),
        )
        .await
        .map_err(|_| {
            ActionError::Invalid(
                "settings options timed out while waiting for the server; lookup cancelled".into(),
            )
        })?
    }

    async fn settings_options_inner(
        &self,
        channel_id: u64,
        reference: Option<&str>,
        field: Option<&str>,
    ) -> Result<ActionResult, ActionError> {
        let target = if reference.is_some() || matches!(field, Some("effort" | "reasoning")) {
            Some(self.resolve_thread(channel_id, reference)?)
        } else {
            None
        };
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let generation = server.generation();
        super::settings_action::ready(server, generation).await?;
        let mut catalog = server.execute(list_models(), Some(generation)).await?;
        let mut reserve_listed = false;
        // Optional quota discovery must not turn an unavailable Reserve read into a normal-model outage.
        if matches!(field, None | Some("model"))
            && let Ok(Ok(rates)) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                server.execute(rate_limits(), Some(generation)),
            )
            .await
            && let Ok(extended) = reserve::catalog(&catalog, &rates)
        {
            catalog = extended;
            reserve_listed = true;
        }
        let mut text = if let Some(target) = &target {
            if matches!(field, Some("effort" | "reasoning")) {
                let observation = server.observed_thread_settings(&target.id, generation)?;
                let (model, source) = if let Some((_, value)) = &observation {
                    (
                        value
                            .get("model")
                            .and_then(serde_json::Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .ok_or_else(|| {
                                ActionError::Invalid(
                                    "observed model is malformed; no effort options confirmed"
                                        .into(),
                                )
                            })?,
                        "마지막 서버 확인 모델",
                    )
                } else {
                    (
                        target.model.as_str(),
                        "마지막 저장 모델 · 현재 실행값 미확인",
                    )
                };
                if model == reserve::MODEL {
                    let rates = server.execute(rate_limits(), Some(generation)).await?;
                    catalog = reserve::catalog(&catalog, &rates)?;
                }
                format!(
                    "대화: {}\n{source}: {model}\n{}",
                    target.id,
                    super::model_catalog::effort_options(&catalog, model)?
                )
            } else {
                format!(
                    "대화: {}\n{}",
                    target.id,
                    super::model_catalog::options(&catalog, field)?
                )
            }
        } else {
            super::model_catalog::options(&catalog, field)?
        };
        if reserve_listed {
            text.push_str("\nLuna Reserve: !settings --model reserve (gpt-reserve, standard; 일반 Luna와 별도)");
        }
        super::settings_action::ready(server, generation).await?;
        if reference.is_none()
            && let Some(target) = target
            && self.target(channel_id)?.0 != target.id
        {
            return Err(ActionError::Invalid(
                "settings option target changed".into(),
            ));
        }
        Ok(immediate(text))
    }

    pub(super) async fn restart_codex(&self) -> Result<ActionResult, ActionError> {
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let restarted = server.force_restart_if_quiescent().await?;
        Ok(immediate(if restarted {
            "Resident Codex app-server restarted.".to_owned()
        } else {
            "Codex app-server restart is pending until active turns and approvals finish."
                .to_owned()
        }))
    }

    pub(super) fn force_restart_codex() -> Result<ActionResult, ActionError> {
        #[cfg(windows)]
        {
            let executable = std::env::current_exe()?;
            let root = std::env::current_dir()?;
            if executable.canonicalize()?
                != root.join("target/release/cdr-runtime.exe").canonicalize()?
            {
                return Err(ActionError::Invalid(
                    "force restart is only supported for this installed Windows bridge".into(),
                ));
            }
            let identity = cdr_windows_native::current_process_identity()
                .map_err(|error| ActionError::Invalid(error.to_string()))?;
            spawn_force_restart(&root, &identity)?;
            Ok(immediate(
                "강제 재시작을 요청했습니다. 진행 중인 작업과 승인 대기를 중단하고 봇·앱서버를 다시 시작합니다.",
            ))
        }
        #[cfg(not(windows))]
        Err(ActionError::Invalid(
            "force restart is currently supported only on Windows".into(),
        ))
    }

    pub(super) async fn resources(&self) -> Result<ActionResult, ActionError> {
        let lifecycle = if let Some(server) = &self.server {
            format!("{:?}", server.lifecycle_snapshot().await)
        } else {
            "unavailable".into()
        };
        let path = self
            .state_db
            .parent()
            .ok_or_else(|| ActionError::Invalid("resource disk path has no parent".into()))?
            .to_owned();
        let host = crate::resource_report::report(path)
            .await
            .map_err(ActionError::Invalid)?;
        let runners = crate::diagnostic_report::queue_report(self.mirror_db.clone())
            .await
            .unwrap_or_else(|error| format!("runner 조회 실패: {error}"));
        Ok(immediate(format!(
            "Rust runtime resources\nruntime_pid: {}\napp_server: {lifecycle}\n{host}\n{runners}",
            std::process::id()
        )))
    }

    pub(super) async fn host_reboot(&self) -> Result<ActionResult, ActionError> {
        if !self.host_commands {
            return Err(ActionError::Invalid(
                "host commands are disabled by DISCORD_ENABLE_HOST_COMMANDS".into(),
            ));
        }
        if !cfg!(windows) {
            return Err(ActionError::Invalid(
                "host reboot is currently supported only on Windows".into(),
            ));
        }
        let status = tokio::process::Command::new("shutdown.exe")
            .args([
                "/r",
                "/t",
                "5",
                "/c",
                "Codex Discord Remote requested restart",
            ])
            .status()
            .await?;
        if !status.success() {
            return Err(ActionError::Invalid(format!(
                "shutdown.exe returned {status}"
            )));
        }
        Ok(immediate("Windows host restart scheduled in 5 seconds."))
    }
}

#[cfg(windows)]
fn spawn_force_restart(root: &std::path::Path, identity: &str) -> Result<(), ActionError> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let script = root.join("codex-discord-rust-restart.ps1");
    if !script.is_file() {
        return Err(ActionError::Invalid(
            "force restart controller is missing".into(),
        ));
    }
    // The script binds this exact process, then hands off via Windows to escape
    // the runtime's job tree. No app-server request or graceful drain is awaited.
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(script)
        .arg("-RepoRoot")
        .arg(root)
        .arg("-Force")
        .arg("-ExpectedBotIdentity")
        .arg(identity)
        .current_dir(root)
        .creation_flags(0x0800_0000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(all(test, windows))]
mod force_tests {
    #[test]
    fn force_launch_passes_exact_identity_without_an_app_server_or_queue_wait() {
        let root = tempfile::Builder::new()
            .prefix("cdr force command ")
            .tempdir()
            .unwrap();
        std::fs::write(root.path().join("codex-discord-rust-restart.ps1"), r"
param($RepoRoot,[switch]$Force,$ExpectedBotIdentity)
[IO.File]::WriteAllText((Join-Path $RepoRoot 'received.json'),(@{root=$RepoRoot;force=[bool]$Force;identity=$ExpectedBotIdentity}|ConvertTo-Json -Compress))
").unwrap();
        let identity = cdr_windows_native::current_process_identity().unwrap();
        super::spawn_force_restart(root.path(), &identity).unwrap();
        let receipt = root.path().join("received.json");
        for _ in 0..100 {
            if receipt.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let received: serde_json::Value =
            serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
        assert_eq!(received["identity"], identity);
        assert_eq!(received["root"], root.path().to_str().unwrap());
        assert_eq!(received["force"], true);
    }
}
