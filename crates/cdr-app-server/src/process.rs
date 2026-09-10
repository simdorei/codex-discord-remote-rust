use std::pin::Pin;
#[cfg(windows)]
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::{AppServerConfig, AppServerError};

#[cfg(windows)]
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) type AppServerInput = Pin<Box<dyn AsyncWrite + Send>>;
pub(crate) type AppServerOutput = Pin<Box<dyn AsyncRead + Send>>;

pub(crate) struct SpawnedAppServer {
    pub child: AppServerProcess,
    pub process_id: Option<u32>,
    pub stdin: AppServerInput,
    pub stdout: AppServerOutput,
    pub stderr: AppServerOutput,
}

pub(crate) enum AppServerProcess {
    #[cfg(windows)]
    Native(cdr_windows_native::CapturedWindowProcess),
    #[cfg(any(not(windows), test))]
    Tokio(tokio::process::Child),
}

pub(crate) fn spawn_app_server(
    config: &AppServerConfig,
) -> Result<SpawnedAppServer, AppServerError> {
    #[cfg(windows)]
    {
        spawn_windows(config)
    }
    #[cfg(not(windows))]
    {
        spawn_portable(config)
    }
}

#[cfg(windows)]
fn spawn_windows(config: &AppServerConfig) -> Result<SpawnedAppServer, AppServerError> {
    let environment = inherited_environment(&config.environment)?;
    let cwd = std::env::current_dir()?;
    let mut child = cdr_windows_native::CapturedWindowProcess::launch_piped(
        &config.executable,
        &config.arguments,
        &cwd,
        &environment,
    )?;
    let process_id = Some(child.process_id());
    let stdin = child
        .take_stdin()
        .ok_or(AppServerError::MissingPipe { stream: "stdin" })?;
    let stdout = child
        .take_stdout()
        .ok_or(AppServerError::MissingPipe { stream: "stdout" })?;
    let stderr = child
        .take_stderr()
        .ok_or(AppServerError::MissingPipe { stream: "stderr" })?;
    Ok(SpawnedAppServer {
        child: AppServerProcess::Native(child),
        process_id,
        stdin: Box::pin(tokio::fs::File::from_std(stdin)),
        stdout: Box::pin(tokio::fs::File::from_std(stdout)),
        stderr: Box::pin(tokio::fs::File::from_std(stderr)),
    })
}

#[cfg(windows)]
fn inherited_environment(
    overrides: &std::collections::BTreeMap<String, String>,
) -> Result<std::collections::HashMap<String, String>, AppServerError> {
    let mut environment = std::collections::HashMap::new();
    for (name, value) in std::env::vars_os() {
        let name = name
            .into_string()
            .map_err(|_| AppServerError::EnvironmentEncoding { field: "name" })?;
        let value = value
            .into_string()
            .map_err(|_| AppServerError::EnvironmentEncoding { field: "value" })?;
        environment.insert(name, value);
    }
    for (name, value) in overrides {
        if let Some(inherited) = environment
            .keys()
            .find(|inherited| inherited.eq_ignore_ascii_case(name))
            .cloned()
        {
            environment.remove(&inherited);
        }
        environment.insert(name.clone(), value.clone());
    }
    Ok(environment)
}

#[cfg(not(windows))]
fn spawn_portable(config: &AppServerConfig) -> Result<SpawnedAppServer, AppServerError> {
    use std::process::Stdio;

    let mut command = tokio::process::Command::new(&config.executable);
    command
        .args(&config.arguments)
        .envs(&config.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|source| AppServerError::Spawn {
        executable: config.executable.display().to_string(),
        source,
    })?;
    let process_id = child.id();
    let stdin = child
        .stdin
        .take()
        .ok_or(AppServerError::MissingPipe { stream: "stdin" })?;
    let stdout = child
        .stdout
        .take()
        .ok_or(AppServerError::MissingPipe { stream: "stdout" })?;
    let stderr = child
        .stderr
        .take()
        .ok_or(AppServerError::MissingPipe { stream: "stderr" })?;
    Ok(SpawnedAppServer {
        child: AppServerProcess::Tokio(child),
        process_id,
        stdin: Box::pin(stdin),
        stdout: Box::pin(stdout),
        stderr: Box::pin(stderr),
    })
}

impl AppServerProcess {
    pub(crate) async fn wait(&mut self) -> Result<(), AppServerError> {
        match self {
            #[cfg(windows)]
            Self::Native(child) => loop {
                if child.try_wait()?.is_some() {
                    return Ok(());
                }
                tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
            },
            #[cfg(any(not(windows), test))]
            Self::Tokio(child) => {
                child.wait().await?;
                Ok(())
            }
        }
    }

    pub(crate) fn start_kill(&mut self) -> Result<(), AppServerError> {
        match self {
            #[cfg(windows)]
            Self::Native(child) => child.start_kill().map_err(Into::into),
            #[cfg(any(not(windows), test))]
            Self::Tokio(child) => child.start_kill().map_err(Into::into),
        }
    }

    pub(crate) fn try_wait(&mut self) -> Result<Option<()>, AppServerError> {
        match self {
            #[cfg(windows)]
            Self::Native(child) => child
                .try_wait()
                .map(|status| status.map(|_| ()))
                .map_err(Into::into),
            #[cfg(any(not(windows), test))]
            Self::Tokio(child) => child
                .try_wait()
                .map(|status| status.map(|_| ()))
                .map_err(Into::into),
        }
    }
}

#[cfg(test)]
impl From<tokio::process::Child> for AppServerProcess {
    fn from(child: tokio::process::Child) -> Self {
        Self::Tokio(child)
    }
}
