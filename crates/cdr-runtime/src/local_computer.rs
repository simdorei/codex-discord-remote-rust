//! Register the installed official CUA runtime for this app-server only.
//!
//! The Discord process no longer implements a Computer Use MCP. Native operations
//! run through the official CUA service inside a real Codex turn, with that turn's
//! permissions and the normal app-approval flow.

pub mod probe;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use cdr_app_server::{AppServerConfig, AppServerError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use crate::config::HEADLESS_COMPUTER_USE_ENV as ENABLED_ENV;
pub const PROVIDER: &str = "cua_repl";
const MAX_MANIFEST_BYTES: u64 = 1_048_576;
const CACHE: &str = "plugins/cache/openai-bundled/unified-computer-use";

#[derive(Clone, Debug, Serialize)]
pub struct OfficialCuaSelection {
    pub manifest: PathBuf,
    pub manifest_sha256: String,
    pub plugin_version: String,
    pub command: PathBuf,
    pub cli: PathBuf,
    pub node_repl: PathBuf,
    pub surfaces: String,
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "mcpServers")]
    servers: BTreeMap<String, Server>,
}

#[derive(Deserialize)]
struct Server {
    command: String,
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    env_vars: Vec<String>,
}

pub fn enabled(environment: &BTreeMap<String, String>) -> Result<bool, AppServerError> {
    crate::config::parse_headless_computer_use(environment)
        .map_err(|error| AppServerError::StartupHeld(error.to_string()))
}

/// Resolve the newest installed official bundle on each runtime start.
/// `false` preserves the previous opt-out semantics: no managed registration;
/// independently configured official plugins are not disabled.
pub fn configure_current(
    config: &mut AppServerConfig,
    computer_enabled: bool,
) -> Result<Option<OfficialCuaSelection>, AppServerError> {
    if !computer_enabled {
        let legacy = legacy_disabled_setting(&config.executable)?;
        config.arguments.extend(["-c".into(), legacy]);
        return Ok(None);
    }
    let home = config
        .environment
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("CODEX_HOME"))
        .map(|(_, value)| PathBuf::from(value))
        .ok_or_else(|| held("official CUA requires the resolved CODEX_HOME"))?;
    configure(config, &newest_manifest(&home)?).map(Some)
}

fn newest_manifest(home: &Path) -> Result<PathBuf, AppServerError> {
    if !home.is_absolute() {
        return Err(held("official CUA requires an absolute CODEX_HOME"));
    }
    let cache = home.join(CACHE);
    let entries = fs::read_dir(&cache).map_err(|error| {
        held(format!(
            "official CUA is not installed or its cache is unreadable at {}: {error}",
            cache.display()
        ))
    })?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| held(format!("official CUA cache: {error}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(version) = numeric_version(&name) else {
            continue;
        };
        if !entry
            .file_type()
            .map_err(|error| held(error.to_string()))?
            .is_dir()
        {
            continue;
        }
        candidates.push((version, entry.path()));
        if candidates.len() > 256 {
            return Err(held("official CUA cache contains too many versions"));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let Some((version, directory)) = candidates.first() else {
        return Err(held("no versioned official CUA installation was found"));
    };
    if candidates
        .get(1)
        .is_some_and(|candidate| &candidate.0 == version)
    {
        return Err(held("official CUA installation version is ambiguous"));
    }
    // A partially installed newest bundle is an error, not permission to silently
    // pin the process to an older version.
    Ok(directory.join(".mcp.json"))
}

fn numeric_version(value: &str) -> Option<Vec<u64>> {
    let parts = value.split('.').collect::<Vec<_>>();
    if !(2..=6).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    parts.into_iter().map(|part| part.parse().ok()).collect()
}

/// Validate a coherent installed manifest before mutating the launch config.
pub fn configure(
    config: &mut AppServerConfig,
    manifest_path: &Path,
) -> Result<OfficialCuaSelection, AppServerError> {
    let manifest_path = absolute_file(manifest_path, "official CUA manifest")?;
    let (mut server, manifest_sha256) = read_manifest(&manifest_path)?;
    let selection = validate_bundle(&manifest_path, &server, manifest_sha256)?;
    let settings = native_settings(&mut server, &config.executable)?;
    for setting in settings {
        config.arguments.extend(["-c".into(), setting]);
    }
    Ok(selection)
}

fn read_manifest(manifest_path: &Path) -> Result<(Server, String), AppServerError> {
    let mut bytes = Vec::new();
    File::open(manifest_path)
        .map_err(|error| held(error.to_string()))?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| held(error.to_string()))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(held("official CUA manifest exceeds 1 MiB"));
    }
    let mut manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| held(format!("invalid official CUA manifest: {error}")))?;
    let server = manifest
        .servers
        .remove(PROVIDER)
        .ok_or_else(|| held("official CUA manifest has no cua_repl server"))?;
    Ok((server, hex::encode(Sha256::digest(&bytes))))
}

fn validate_bundle(
    manifest_path: &Path,
    server: &Server,
    manifest_sha256: String,
) -> Result<OfficialCuaSelection, AppServerError> {
    let command = absolute_file(Path::new(&server.command), "official CUA Node executable")?;
    let bin = command
        .parent()
        .ok_or_else(|| held("official CUA Node directory is missing"))?;
    if command.file_name().and_then(|name| name.to_str()) != Some("node.exe") {
        return Err(held("official CUA manifest must use its bundled node.exe"));
    }
    let node_repl = required_file(&server.env, "CUA_REPL_NODE_REPL_PATH")?;
    if !same_path(node_repl.parent().unwrap_or(Path::new("")), bin)? {
        return Err(held(
            "official CUA Node and node_repl must belong to the same bundle",
        ));
    }
    let cli = required_file(&server.env, "CODEX_CLI_PATH")?;
    if let Some(node) = server.env.get("NODE_REPL_NODE_PATH")
        && !same_path(
            &absolute_file(Path::new(node), "NODE_REPL_NODE_PATH")?,
            &command,
        )?
    {
        return Err(held("official CUA Node paths do not match"));
    }
    let plugin_dir = manifest_path
        .parent()
        .ok_or_else(|| held("official CUA plugin directory is missing"))?;
    let (entry, guard) = validate_launcher(&server.args, bin, plugin_dir)?;
    let helper_name = if cfg!(target_arch = "aarch64") {
        "codex-computer-use-arm64.exe"
    } else {
        "codex-computer-use.exe"
    };
    let helper = absolute_file(
        &bin.join("node_modules/@oai/sky/bin/windows")
            .join(helper_name),
        "official native Computer Use helper",
    )?;
    if server
        .env
        .iter()
        .any(|(key, value)| key.contains('\0') || value.contains('\0'))
        || server
            .env_vars
            .iter()
            .any(|key| key.is_empty() || key.contains(['\0', '=']))
    {
        return Err(held("invalid official CUA environment"));
    }
    // Keep validation atomic without holding shared installation files open.
    // The selected outer Codex executable retains its separately managed lease.
    for path in [&command, &node_repl, &cli, &entry, &guard, &helper] {
        File::open(path).map_err(|error| {
            held(format!(
                "official CUA artifact is unreadable at {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(OfficialCuaSelection {
        manifest: manifest_path.to_owned(),
        manifest_sha256,
        plugin_version: plugin_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .into(),
        command,
        cli,
        node_repl,
        surfaces: "computer".into(),
    })
}

fn validate_launcher(
    arguments: &[String],
    bin: &Path,
    plugin_dir: &Path,
) -> Result<(PathBuf, PathBuf), AppServerError> {
    let entry = absolute_file(
        &bin.join("node_modules/@oai/cua-repl/bin/cua-repl.mjs"),
        "official CUA entry point",
    )?;
    let guard = absolute_file(
        &plugin_dir.join("require-browser-access.mjs"),
        "official CUA access guard",
    )?;
    let [import, guard_url, entry_arg] = arguments else {
        return Err(held(
            "unrecognized official CUA launcher arguments; refusing to replace its access guard",
        ));
    };
    let supplied_entry = absolute_file(Path::new(entry_arg), "official CUA manifest entry point")?;
    let parsed_guard = reqwest::Url::parse(guard_url)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| held("official CUA access guard must be a file URL"))?;
    if import != "--import"
        || !same_path(&supplied_entry, &entry)?
        || !same_path(&parsed_guard, &guard)?
    {
        return Err(held(
            "official CUA entry point or access guard does not match its bundle",
        ));
    }
    Ok((entry, guard))
}

fn native_settings(
    server: &mut Server,
    codex_executable: &Path,
) -> Result<Vec<String>, AppServerError> {
    // Preserve the installed guard, trusted services, CLI pairing and approvals.
    // Only the native transport and exposed surface change.
    server.env.insert("SKY_CUA_NATIVE_PIPE".into(), "0".into());
    server
        .env
        .insert("SKY_CUA_NATIVE_PIPE_DIRECTORY".into(), String::new());
    server
        .env
        .insert("CUA_REPL_ENABLED_SURFACES".into(), "computer".into());
    server.env_vars.retain(|key| {
        !key.eq_ignore_ascii_case("SKY_CUA_NATIVE_PIPE_DIRECTORY")
            && !key.eq_ignore_ascii_case("SKY_CUA_NATIVE_PIPE")
            && !key.eq_ignore_ascii_case("CUA_REPL_ENABLED_SURFACES")
    });
    let environment = format!(
        "{{{}}}",
        server
            .env
            .iter()
            .map(|(key, value)| format!("{}={}", quote(key), quote(value)))
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(vec![
        legacy_disabled_setting(codex_executable)?,
        format!("mcp_servers.{PROVIDER}.command={}", quote(&server.command)),
        format!(
            "mcp_servers.{PROVIDER}.args={}",
            serde_json::to_string(&server.args).map_err(|e| held(e.to_string()))?
        ),
        format!("mcp_servers.{PROVIDER}.env={environment}"),
        format!(
            "mcp_servers.{PROVIDER}.env_vars={}",
            serde_json::to_string(&server.env_vars).map_err(|e| held(e.to_string()))?
        ),
        format!("mcp_servers.{PROVIDER}.enabled=true"),
        format!("mcp_servers.{PROVIDER}.startup_timeout_sec=30"),
        format!("mcp_servers.{PROVIDER}.tool_timeout_sec=120"),
    ])
}

fn legacy_disabled_setting(executable: &Path) -> Result<String, AppServerError> {
    let executable = executable
        .to_str()
        .ok_or_else(|| held("selected Codex executable path is not UTF-8"))?;
    Ok(format!(
        r#"mcp_servers.cdr_computer={{enabled=false,command={},args=["--version"]}}"#,
        quote(executable)
    ))
}

fn held(message: impl Into<String>) -> AppServerError {
    AppServerError::StartupHeld(message.into())
}

fn quote(value: &str) -> String {
    // This becomes a direct argv value, never shell input.
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

fn absolute_file(path: &Path, label: &str) -> Result<PathBuf, AppServerError> {
    if !path.is_absolute() || !path.is_file() {
        return Err(held(format!(
            "{label} is missing or not an absolute file: {}",
            path.display()
        )));
    }
    if path.to_str().is_none() {
        return Err(held(format!("{label} is not UTF-8")));
    }
    Ok(path.to_owned())
}

fn required_file(env: &BTreeMap<String, String>, key: &str) -> Result<PathBuf, AppServerError> {
    absolute_file(
        Path::new(
            env.get(key)
                .ok_or_else(|| held(format!("official CUA manifest lacks {key}")))?,
        ),
        key,
    )
}

fn same_path(left: &Path, right: &Path) -> Result<bool, AppServerError> {
    Ok(
        fs::canonicalize(left).map_err(|error| held(error.to_string()))?
            == fs::canonicalize(right).map_err(|error| held(error.to_string()))?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(root: &Path, version: &str) -> PathBuf {
        let plugin = root.join(CACHE).join(version);
        let bin = root.join("official runtime").join(version).join("bin");
        let node = bin.join("node.exe");
        let repl = bin.join("node_repl.exe");
        let cli = root.join("official cli").join(version).join("codex.exe");
        let entry = bin.join("node_modules/@oai/cua-repl/bin/cua-repl.mjs");
        let guard = plugin.join("require-browser-access.mjs");
        let helper_name = if cfg!(target_arch = "aarch64") {
            "codex-computer-use-arm64.exe"
        } else {
            "codex-computer-use.exe"
        };
        let helper = bin
            .join("node_modules/@oai/sky/bin/windows")
            .join(helper_name);
        for file in [&node, &repl, &cli, &entry, &guard, &helper] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, "fixture").unwrap();
        }
        let path = plugin.join(".mcp.json");
        fs::write(&path, serde_json::to_vec(&json!({"mcpServers":{"cua_repl":{
            "command":node, "args":["--import",reqwest::Url::from_file_path(&guard).unwrap().to_string(),entry],
            "env":{"CODEX_CLI_PATH":cli,"CUA_REPL_NODE_REPL_PATH":repl,"NODE_REPL_NODE_PATH":node,
                "SKY_CUA_NATIVE_PIPE":"1","SKY_CUA_NATIVE_PIPE_DIRECTORY":"old-desktop-pipe",
                "CUA_REPL_ENABLED_SURFACES":"browser,computer",
                "NODE_REPL_TRUSTED_SERVICES":"{\"sky\":\"@oai/sky/service\",\"browser\":\"@oai/browser-desktop/service\"}"},
            "env_vars":["SKY_CUA_NATIVE_PIPE_DIRECTORY","PRESERVED_MANIFEST_ENV"]
        }}})).unwrap()).unwrap();
        path
    }

    #[test]
    fn registers_official_bundle_without_editing_user_settings_or_approvals() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("profile with spaces # and %");
        let manifest = fixture(&home, "26.1106.51428");
        let bytes = fs::read(&manifest).unwrap();
        let mut config = AppServerConfig::new("codex.exe")
            .with_environment([("CODEX_HOME".into(), home.display().to_string())]);
        let existing_lease = std::sync::Arc::new(File::open(&manifest).unwrap());
        config
            .retained_executable_files
            .push(std::sync::Arc::clone(&existing_lease));
        let before = config.arguments.clone();
        let selection = configure_current(&mut config, true).unwrap().unwrap();
        assert_eq!(selection.manifest, manifest);
        assert_eq!(selection.surfaces, "computer");
        assert_eq!(&config.arguments[..before.len()], before);
        let settings = config.arguments[before.len()..]
            .chunks_exact(2)
            .map(|p| p[1].as_str())
            .collect::<Vec<_>>();
        let legacy =
            r#"mcp_servers.cdr_computer={enabled=false,command="codex.exe",args=["--version"]}"#;
        assert!(settings.contains(&legacy));
        assert!(
            settings
                .iter()
                .all(|s| s.starts_with("mcp_servers.cua_repl.") || *s == legacy)
        );
        assert!(!settings.iter().any(|s| s.contains("--computer-mcp")));
        let env = settings
            .iter()
            .find(|s| s.starts_with("mcp_servers.cua_repl.env="))
            .unwrap();
        assert!(env.contains("\"SKY_CUA_NATIVE_PIPE\"=\"0\""));
        assert!(env.contains("\"SKY_CUA_NATIVE_PIPE_DIRECTORY\"=\"\""));
        assert!(env.contains("@oai/sky/service"));
        assert!(!env.contains("old-desktop-pipe"));
        assert!(
            settings
                .iter()
                .any(|s| s.contains("require-browser-access.mjs"))
        );
        assert_eq!(fs::read(&manifest).unwrap(), bytes);
        assert_eq!(config.environment.len(), 1);
        assert_eq!(config.retained_executable_files.len(), 1);
        assert!(std::sync::Arc::ptr_eq(
            &config.retained_executable_files[0],
            &existing_lease
        ));
        // In particular, registration must not prevent a Windows installer from
        // replacing a shared JavaScript guard while this config remains alive.
        let guard = manifest
            .parent()
            .unwrap()
            .join("require-browser-access.mjs");
        let moved_guard = guard.with_extension("mjs.old");
        fs::rename(&guard, &moved_guard).unwrap();
        fs::rename(&moved_guard, &guard).unwrap();
    }

    #[test]
    fn chooses_numeric_latest_and_refuses_an_incomplete_newest_bundle() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path(), "26.9.1");
        let latest = fixture(temp.path(), "26.10.2");
        assert_eq!(newest_manifest(temp.path()).unwrap(), latest);
        let incomplete = temp.path().join(CACHE).join("26.11.1");
        fs::create_dir_all(&incomplete).unwrap();
        let mut config = AppServerConfig::new("codex.exe")
            .with_environment([("CODEX_HOME".into(), temp.path().display().to_string())]);
        let before = config.arguments.clone();
        assert!(configure_current(&mut config, true).is_err());
        assert_eq!(config.arguments, before);
        assert!(config.retained_executable_files.is_empty());
    }

    #[test]
    fn rejects_a_changed_access_guard_without_partial_registration() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = fixture(temp.path(), "26.1106.51428");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value["mcpServers"]["cua_repl"]["args"] = json!(["--eval", "custom helper"]);
        fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut config = AppServerConfig::new("codex.exe");
        let before = config.arguments.clone();
        assert!(configure(&mut config, &manifest).is_err());
        assert_eq!(config.arguments, before);
        assert!(config.retained_executable_files.is_empty());
    }

    #[test]
    fn rejects_a_relative_entry_point_without_partial_registration() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = fixture(temp.path(), "26.1106.51428");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value["mcpServers"]["cua_repl"]["args"][2] =
            json!("node_modules/@oai/cua-repl/bin/cua-repl.mjs");
        fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut config = AppServerConfig::new("codex.exe");
        let before = config.arguments.clone();
        let error = configure(&mut config, &manifest).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("official CUA manifest entry point is missing or not an absolute file")
        );
        assert_eq!(config.arguments, before);
        assert!(config.retained_executable_files.is_empty());
    }

    #[test]
    fn rejects_mixed_runtime_and_plugin_files_without_partial_registration() {
        let temp = tempfile::tempdir().unwrap();
        let older_manifest = fixture(temp.path(), "26.9.1");
        let manifest = fixture(temp.path(), "26.10.2");
        let older: serde_json::Value =
            serde_json::from_slice(&fs::read(older_manifest).unwrap()).unwrap();
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        for pointer in [
            "/mcpServers/cua_repl/env/CUA_REPL_NODE_REPL_PATH",
            "/mcpServers/cua_repl/env/NODE_REPL_NODE_PATH",
            "/mcpServers/cua_repl/args/2",
            "/mcpServers/cua_repl/args/1",
        ] {
            let mut mixed = original.clone();
            *mixed.pointer_mut(pointer).unwrap() = older.pointer(pointer).unwrap().clone();
            fs::write(&manifest, serde_json::to_vec(&mixed).unwrap()).unwrap();
            let mut config = AppServerConfig::new("codex.exe");
            let before = config.arguments.clone();
            assert!(configure(&mut config, &manifest).is_err(), "{pointer}");
            assert_eq!(config.arguments, before, "{pointer}");
            assert!(config.retained_executable_files.is_empty(), "{pointer}");
        }
    }

    #[test]
    fn opt_out_masks_legacy_provider_and_preserves_existing_official_configuration() {
        let mut config = AppServerConfig::new(r"C:\Program Files\Codex\codex.exe");
        config
            .arguments
            .extend(["-c".into(), "mcp_servers.cua_repl.enabled=true".into()]);
        let before = config.arguments.clone();
        assert!(configure_current(&mut config, false).unwrap().is_none());
        assert_eq!(&config.arguments[..before.len()], before);
        assert_eq!(
            &config.arguments[before.len()..],
            [
                "-c",
                r#"mcp_servers.cdr_computer={enabled=false,command="C:\\Program Files\\Codex\\codex.exe",args=["--version"]}"#,
            ]
        );
        assert!(
            !config
                .arguments
                .iter()
                .any(|arg| arg.contains("--computer-mcp"))
        );
    }
}
