//! Opt-in local desktop MCP. This does not modify the bundled CUA plugin.
use std::collections::BTreeMap;
use std::path::Path;

use cdr_app_server::{AppServerConfig, AppServerError};

pub const ENABLED_ENV: &str = "DISCORD_HEADLESS_COMPUTER_USE";

pub fn enabled(environment: &BTreeMap<String, String>) -> Result<bool, AppServerError> {
    match environment.get(ENABLED_ENV).map(|value| value.trim()) {
        None | Some("" | "0" | "false") => Ok(false),
        Some("1" | "true") if cfg!(windows) => Ok(true),
        Some("1" | "true") => Err(AppServerError::StartupHeld(
            "DISCORD_HEADLESS_COMPUTER_USE requires Windows".into(),
        )),
        Some(_) => Err(AppServerError::StartupHeld(
            "DISCORD_HEADLESS_COMPUTER_USE must be 0 or 1".into(),
        )),
    }
}

/// Register only this runtime's explicitly selected stdio endpoint.
/// Existing tool grants, plugins, browser policy, and user configuration stay intact.
pub fn configure(
    config: &mut AppServerConfig,
    runtime_executable: &Path,
) -> Result<(), AppServerError> {
    if !runtime_executable.is_absolute() || !runtime_executable.is_file() {
        return Err(AppServerError::StartupHeld(
            "headless Computer Use requires the current absolute runtime executable".into(),
        ));
    }
    let path = runtime_executable.to_str().ok_or_else(|| {
        AppServerError::StartupHeld("headless Computer Use executable is not UTF-8".into())
    })?;
    // JSON string escaping is compatible with TOML basic strings, including
    // Windows backslashes and spaces. No shell or command interpolation is used.
    let command = serde_json::to_string(path)
        .map_err(|error| AppServerError::StartupHeld(error.to_string()))?;
    for setting in [
        format!("mcp_servers.cdr_computer.command={command}"),
        "mcp_servers.cdr_computer.args=[\"--computer-mcp\"]".into(),
        "mcp_servers.cdr_computer.enabled=true".into(),
        "mcp_servers.cdr_computer.startup_timeout_sec=10".into(),
        "mcp_servers.cdr_computer.tool_timeout_sec=30".into(),
    ] {
        config.arguments.extend(["-c".into(), setting]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_requires_an_explicit_valid_operator_setting() {
        let mut environment = BTreeMap::new();
        assert!(!enabled(&environment).unwrap());
        for value in ["", "0", "false"] {
            environment.insert(ENABLED_ENV.into(), value.into());
            assert!(!enabled(&environment).unwrap());
        }
        environment.insert(ENABLED_ENV.into(), "1".into());
        if cfg!(windows) {
            assert!(enabled(&environment).unwrap());
        } else {
            assert!(enabled(&environment).is_err());
        }
        environment.insert(ENABLED_ENV.into(), "maybe".into());
        assert!(enabled(&environment).is_err());
    }

    #[test]
    fn registers_the_exact_runtime_without_overwriting_existing_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime with spaces.exe");
        std::fs::write(&runtime, b"fixture").unwrap();
        let mut config = AppServerConfig::new("codex.exe")
            .with_environment([("CODEX_HOME".into(), "existing-home".into())]);
        let before = config.arguments.clone();
        configure(&mut config, &runtime).unwrap();
        assert_eq!(&config.arguments[..before.len()], before);
        let settings: Vec<_> = config.arguments[before.len()..]
            .chunks_exact(2)
            .map(|pair| {
                assert_eq!(pair[0], "-c");
                pair[1].as_str()
            })
            .collect();
        let command = settings[0]
            .strip_prefix("mcp_servers.cdr_computer.command=")
            .unwrap();
        assert_eq!(
            serde_json::from_str::<String>(command).unwrap(),
            runtime.to_str().unwrap()
        );
        assert_eq!(
            settings[1],
            "mcp_servers.cdr_computer.args=[\"--computer-mcp\"]"
        );
        assert_eq!(config.environment["CODEX_HOME"], "existing-home");
        assert_eq!(config.environment.len(), 1);
        assert!(
            settings
                .iter()
                .all(|value| value.starts_with("mcp_servers.cdr_computer."))
        );
    }

    #[test]
    fn invalid_executable_does_not_partially_register_a_server() {
        let mut config = AppServerConfig::new("codex.exe");
        let before = config.arguments.clone();
        assert!(configure(&mut config, Path::new("relative.exe")).is_err());
        assert_eq!(config.arguments, before);
        let temp = tempfile::tempdir().unwrap();
        assert!(configure(&mut config, &temp.path().join("missing.exe")).is_err());
        assert_eq!(config.arguments, before);
    }
}
