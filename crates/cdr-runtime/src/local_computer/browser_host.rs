//! Versioned, embedded companion for the official CUA MCP transport.
//!
//! This mode starts no Discord connection or remote-agent worker. Its stdout is
//! reserved for the unchanged official MCP protocol.

mod cache;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cdr_app_server::{AppServerConfig, AppServerError};
use serde::Serialize;

use super::{OfficialCuaSelection, Server, held, read_manifest, validate_bundle};

pub const ENTRYPOINT: &str = "--official-browser-host";

#[derive(Clone, Debug, Serialize)]
pub struct BrowserHostSelection {
    pub mode: &'static str,
    pub runtime: PathBuf,
    pub cache_root: PathBuf,
    pub embedded_sha256: String,
}

pub(super) fn configure(
    config: &AppServerConfig,
    server: &mut Server,
    selection: &mut OfficialCuaSelection,
) -> Result<(), AppServerError> {
    if !selection.surfaces.split(',').any(|s| s.trim() == "browser") {
        return Ok(());
    }
    let home = config
        .environment
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("CODEX_HOME"))
        .map(|(_, home)| PathBuf::from(home))
        .ok_or_else(|| held("owned browser host requires the resolved CODEX_HOME"))?;
    let runtime = std::env::current_exe().map_err(|e| held(e.to_string()))?;
    let cache_root = home.join("cdr-browser-host");
    // Publishing here makes a missing/unwritable cache an ordinary optional-CUA
    // registration failure, before partially modifying the app-server argv.
    cache::prepare(&cache_root)?;
    utf8_path(&runtime)?.clone_into(&mut server.command);
    server.args = vec![
        ENTRYPOINT.into(),
        "--manifest".into(),
        utf8_path(&selection.manifest)?.into(),
        "--expected-manifest-sha256".into(),
        selection.manifest_sha256.clone(),
        "--cache-root".into(),
        utf8_path(&cache_root)?.into(),
    ];
    selection.browser_host = Some(BrowserHostSelection {
        mode: "owned_mcp_call",
        runtime,
        cache_root,
        embedded_sha256: cache::digest(),
    });
    Ok(())
}

fn utf8_path(path: &Path) -> Result<&str, AppServerError> {
    path.to_str()
        .ok_or_else(|| held("browser host path is not UTF-8"))
}

#[derive(Debug)]
struct Arguments {
    manifest: PathBuf,
    digest: String,
    cache_root: PathBuf,
    audit_directory: Option<PathBuf>,
}

fn parse(arguments: &[OsString]) -> Result<Arguments, AppServerError> {
    let mut manifest = None;
    let mut digest = None;
    let mut cache_root = None;
    let mut audit_directory = None;
    if !arguments.len().is_multiple_of(2) {
        return Err(held("incomplete official browser host arguments"));
    }
    for pair in arguments.chunks_exact(2) {
        let destination = match pair[0].to_str() {
            Some("--manifest") => &mut manifest,
            Some("--expected-manifest-sha256") => &mut digest,
            Some("--cache-root") => &mut cache_root,
            Some("--audit-directory") => &mut audit_directory,
            _ => return Err(held("unknown official browser host argument")),
        };
        if destination.replace(pair[1].clone()).is_some() {
            return Err(held("duplicate official browser host argument"));
        }
    }
    let manifest = PathBuf::from(manifest.ok_or_else(|| held("missing official manifest"))?);
    let cache_root = PathBuf::from(cache_root.ok_or_else(|| held("missing browser host cache"))?);
    let audit_directory = audit_directory.map(PathBuf::from);
    let digest = digest
        .and_then(|value| value.into_string().ok())
        .filter(|value| value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| held("missing or invalid selected manifest digest"))?;
    if !manifest.is_absolute()
        || !cache_root.is_absolute()
        || audit_directory.as_ref().is_some_and(|p| !p.is_absolute())
    {
        return Err(held("browser host paths must be absolute"));
    }
    Ok(Arguments {
        manifest,
        digest: digest.to_ascii_lowercase(),
        cache_root,
        audit_directory,
    })
}

/// Internal MCP entrypoint. Called before normal bot environment/DB startup.
pub fn run(arguments: &[OsString]) -> Result<(), AppServerError> {
    if !cfg!(windows) {
        return Err(held("the owned browser host requires Windows"));
    }
    let args = parse(arguments)?;
    let (server, digest) = read_manifest(&args.manifest)?;
    if digest != args.digest {
        return Err(held("official CUA manifest changed after selection"));
    }
    let selection = validate_bundle(&args.manifest, &server, digest)?;
    let launcher = cache::prepare(&args.cache_root)?;
    let mut child = std::process::Command::new(&selection.command);
    child
        .arg(launcher)
        .arg("--manifest")
        .arg(&args.manifest)
        .arg("--expected-manifest-sha256")
        .arg(&args.digest)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    if let Some(audit_directory) = args.audit_directory {
        child.arg("--audit-directory").arg(audit_directory);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let status = child.status()?;
    if !status.success() {
        return Err(held(format!("official browser host exited: {status}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_duplicate_and_relative_startup_arguments() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["--manifest".into()]).is_err());
        let temp = tempfile::tempdir().unwrap();
        let mut args = vec![
            "--manifest".into(),
            temp.path().join(".mcp.json").into_os_string(),
            "--expected-manifest-sha256".into(),
            "a".repeat(64).into(),
            "--cache-root".into(),
            temp.path().join("cache").into_os_string(),
        ];
        assert!(parse(&args).is_ok());
        args.extend(["--manifest".into(), "relative.json".into()]);
        assert!(parse(&args).is_err());
        args.truncate(6);
        args[1] = "relative.json".into();
        assert!(parse(&args).is_err());
    }

    #[test]
    #[cfg(windows)]
    fn changed_manifest_is_rejected_before_any_child_launch() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join(".mcp.json");
        std::fs::write(
            &manifest,
            br#"{"mcpServers":{"cua_repl":{"command":"","args":[]}}}"#,
        )
        .unwrap();
        let error = run(&[
            "--manifest".into(),
            manifest.into_os_string(),
            "--expected-manifest-sha256".into(),
            "0".repeat(64).into(),
            "--cache-root".into(),
            temp.path().join("cache").into_os_string(),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("changed after selection"));
        assert!(!temp.path().join("cache").exists());
    }
}
