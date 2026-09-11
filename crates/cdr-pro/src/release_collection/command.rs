use super::{Outcome, Runner, collect};
use crate::{
    fingerprint::fingerprint_required_plugins,
    inventory::read_codex_plugin_inventory,
    preflight::{ResidentSnapshot, expected_remote_plugin_version, verify_runtime},
};
use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

pub struct RealRunner {
    runtime: tokio::runtime::Runtime,
}
impl RealRunner {
    pub fn new() -> Result<Self, String> {
        tokio::runtime::Runtime::new()
            .map(|runtime| Self { runtime })
            .map_err(|e| e.to_string())
    }
}
impl Runner for RealRunner {
    fn command(&mut self, args: &[&str], root: &Path) -> Outcome {
        match self.runtime.block_on(execute(args, root)) {
            Ok(outcome) => outcome,
            Err(error) => Outcome {
                code: -1,
                stdout: String::new(),
                stderr: error,
            },
        }
    }
    fn fresh_resident(&mut self, executable: &Path, manifest: &Path) -> Result<(), String> {
        self.runtime.block_on(resident(executable, manifest))
    }
}
async fn read_output(reader: impl AsyncRead + Unpin) -> Result<Vec<u8>, String> {
    const MAX: u64 = 64 * 1024 * 1024;
    let mut bytes = Vec::new();
    reader
        .take(MAX + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX {
        return Err("release command output exceeded 64 MiB".into());
    }
    Ok(bytes)
}
async fn execute(args: &[&str], root: &Path) -> Result<Outcome, String> {
    let mut command = Command::new(args.first().ok_or("empty release command")?);
    command
        .args(&args[1..])
        .current_dir(root)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("release command could not start: {e}"))?;
    let stdout = child.stdout.take().ok_or("missing stdout pipe")?;
    let stderr = child.stderr.take().ok_or("missing stderr pipe")?;
    let (stdout, stderr, status) = tokio::time::timeout(Duration::from_mins(10), async {
        tokio::try_join!(read_output(stdout), read_output(stderr), async {
            child.wait().await.map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|_| "release command timed out after 600 seconds")??;
    Ok(Outcome {
        code: status.code().unwrap_or(-1),
        stdout: String::from_utf8(stdout).map_err(|e| e.to_string())?,
        stderr: String::from_utf8(stderr).map_err(|e| e.to_string())?,
    })
}
async fn resident(executable: &Path, manifest: &Path) -> Result<(), String> {
    let inventory = read_codex_plugin_inventory(executable)
        .await
        .map_err(|e| e.to_string())?;
    let baseline = fingerprint_required_plugins(&inventory).map_err(|e| e.to_string())?;
    let expected = expected_remote_plugin_version(manifest).map_err(|e| e.to_string())?;
    let server = tokio::time::timeout(
        Duration::from_secs(20),
        ResidentAppServer::start(AppServerConfig::new(executable)),
    )
    .await
    .map_err(|_| "fresh resident startup timed out")?
    .map_err(|e| e.to_string())?;
    let result = async {
        let inventory = read_codex_plugin_inventory(executable)
            .await
            .map_err(|e| e.to_string())?;
        let current = fingerprint_required_plugins(&inventory).map_err(|e| e.to_string())?;
        let life = server.lifecycle_snapshot().await;
        let snapshot = ResidentSnapshot {
            generation: life.generation,
            healthy: life.healthy,
            accepting: life.healthy,
            plugin_runtime_fingerprint: Some(baseline),
            plugin_runtime_error: None,
        };
        verify_runtime(&inventory, &expected, &snapshot, &current)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    .await;
    let close = tokio::time::timeout(Duration::from_secs(12), server.close())
        .await
        .map_err(|_| "fresh resident close timed out".to_owned())
        .and_then(|v| v.map_err(|e| e.to_string()));
    match (result, close) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(e), Ok(())) | (Ok(()), Err(e)) => Err(e),
        (Err(e), Err(close)) => Err(format!("{e}; close also failed: {close}")),
    }
}

pub fn run_cli(args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut args = args;
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        if !["--repo-root", "--output", "--codex-exe"].contains(&key.as_str()) {
            return Err(format!("unknown release collector option: {key}"));
        }
        let value = args
            .next()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("missing value after {key}"))?;
        if options.insert(key.clone(), value).is_some() {
            return Err(format!("duplicate release collector option: {key}"));
        }
    }
    let root = options
        .get("--repo-root")
        .map_or_else(
            || std::env::current_dir().map_err(|e| e.to_string()),
            |p| Ok(PathBuf::from(p)),
        )?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let output = root.join(options.get("--output").map_or(
        ".release-evidence/pro-release-evidence.json",
        String::as_str,
    ));
    let codex = options.get("--codex-exe").map_or("codex", String::as_str);
    let collection = collect(&root, codex, &mut RealRunner::new()?);
    collection.evidence.write(&output)?;
    print!("{}", collection.evidence.summary());
    for problem in collection.problems {
        eprintln!("{}", redact_environment_secrets(&problem));
    }
    println!("JSON: {}", output.display());
    if !collection.evidence.pre_restart_ready {
        return Err(
            "Pro pre-restart evidence is not ready; see the recorded failing checks".into(),
        );
    }
    Ok(())
}

fn redact_environment_secrets(input: &str) -> String {
    let mut output = input.to_owned();
    for (key, value) in std::env::vars_os() {
        let key = key.to_string_lossy().to_uppercase();
        if ["TOKEN", "SECRET", "PASSWORD", "API_KEY"]
            .iter()
            .any(|part| key.contains(part))
        {
            let value = value.to_string_lossy();
            if !value.is_empty() {
                output = output.replace(value.as_ref(), "[REDACTED]");
            }
        }
    }
    output.chars().take(4000).collect()
}
