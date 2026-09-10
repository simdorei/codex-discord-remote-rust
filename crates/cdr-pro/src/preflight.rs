use serde_json::{Map, Value};

use crate::diagnostics::{DiagnosticCode, DiagnosticStage, ProError, Result, RuntimeDiagnostic};

mod manifest;
mod recovery;
pub use manifest::expected_remote_plugin_version;
pub use recovery::recover_stale;

pub const REMOTE_PLUGIN_ID: &str = "codex-discord-remote@codex-discord-remote";
pub const CHROME_PLUGIN_ID: &str = "chrome@openai-bundled";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginStatus {
    pub remote_version: String,
    pub browser_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidentSnapshot {
    pub generation: u64,
    pub healthy: bool,
    pub accepting: bool,
    pub plugin_runtime_fingerprint: Option<String>,
    pub plugin_runtime_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStatus {
    pub remote_plugin_version: String,
    pub browser_plugin_version: String,
    pub resident_generation: u64,
}

pub fn verify_runtime(
    inventory_json: &str,
    expected_remote_version: &str,
    resident: &ResidentSnapshot,
    current_plugin_fingerprint: &str,
) -> Result<RuntimeStatus> {
    let plugins = verify_plugin_inventory(inventory_json, expected_remote_version)?;
    if !resident.healthy || !resident.accepting {
        return Err(failure(
            DiagnosticStage::ResidentAppServer,
            DiagnosticCode::ResidentUnhealthy,
            "The resident Codex process is not ready to accept !pro.",
            "Restart the remote bot, wait for it to become healthy, then retry !pro.",
            format!(
                "resident Codex app-server is not healthy (generation {})",
                resident.generation
            ),
        ));
    }
    if let Some(error) = &resident.plugin_runtime_error {
        return Err(failure(
            DiagnosticStage::ResidentAppServer,
            DiagnosticCode::ResidentSnapshotFailed,
            "The resident Codex process could not verify its plugin snapshot.",
            "Repair the plugin installation and restart the remote bot.",
            format!("resident Codex app-server plugin snapshot failed: {error}"),
        ));
    }
    let Some(fingerprint) = &resident.plugin_runtime_fingerprint else {
        return Err(failure(
            DiagnosticStage::ResidentAppServer,
            DiagnosticCode::ResidentSnapshotMissing,
            "The resident Codex process started without a verified plugin snapshot.",
            "Restart the remote bot, then retry !pro.",
            format!(
                "resident Codex app-server has no plugin snapshot (generation {})",
                resident.generation
            ),
        ));
    };
    if fingerprint != current_plugin_fingerprint {
        return Err(failure(
            DiagnosticStage::ResidentAppServer,
            DiagnosticCode::ResidentStale,
            "The installed plugins changed after the resident Codex process started.",
            "Restart the remote bot, then retry !pro.",
            format!(
                "resident Codex app-server plugin snapshot is stale (generation {}); restart the remote bot",
                resident.generation
            ),
        ));
    }
    Ok(RuntimeStatus {
        remote_plugin_version: plugins.remote_version,
        browser_plugin_version: plugins.browser_version,
        resident_generation: resident.generation,
    })
}

pub fn verify_plugin_inventory(raw: &str, expected_remote_version: &str) -> Result<PluginStatus> {
    let inventory: Value = serde_json::from_str(raw).map_err(|error| {
        invalid_inventory(format!("Codex plugin inventory is not valid JSON: {error}"))
    })?;
    let object = inventory
        .as_object()
        .ok_or_else(|| invalid_inventory("Codex plugin inventory must be a JSON object".into()))?;
    let records = object
        .get("installed")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            invalid_inventory("Codex plugin inventory.installed must be a JSON array".into())
        })?;
    if records.iter().any(|record| !record.is_object()) {
        return Err(invalid_inventory(
            "Codex plugin inventory entries must be JSON objects".into(),
        ));
    }
    let remote = required_plugin(records, REMOTE_PLUGIN_ID, false)?;
    let browser = required_plugin(records, CHROME_PLUGIN_ID, true)?;
    let remote_version = enabled_version(remote, REMOTE_PLUGIN_ID, false)?;
    let browser_version = enabled_version(browser, CHROME_PLUGIN_ID, true)?;
    if remote_version != expected_remote_version {
        return Err(failure(
            DiagnosticStage::PluginInventory,
            DiagnosticCode::RemotePluginVersionMismatch,
            "The installed remote plugin version does not match this bot.",
            "Reinstall the remote plugin and restart the remote bot.",
            format!(
                "plugin '{REMOTE_PLUGIN_ID}' version mismatch: expected '{expected_remote_version}', got '{remote_version}'"
            ),
        ));
    }
    Ok(PluginStatus {
        remote_version,
        browser_version,
    })
}

fn required_plugin<'a>(
    records: &'a [Value],
    plugin_id: &str,
    browser: bool,
) -> Result<&'a Map<String, Value>> {
    let matches = records
        .iter()
        .filter_map(Value::as_object)
        .filter(|record| record.get("pluginId").and_then(Value::as_str) == Some(plugin_id))
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        return Ok(matches[0]);
    }
    Err(failure(
        DiagnosticStage::PluginInventory,
        if browser {
            DiagnosticCode::BrowserPluginMissing
        } else {
            DiagnosticCode::RemotePluginMissing
        },
        if browser {
            "The Chrome plugin entry is missing or duplicated; Chrome availability was not tested."
        } else {
            "The remote plugin entry is missing or duplicated."
        },
        if browser {
            "Reinstall and enable the Chrome plugin, then retry !pro."
        } else {
            "Reinstall and enable the remote plugin, then retry !pro."
        },
        format!("plugin '{plugin_id}' was not installed exactly once"),
    ))
}

fn enabled_version(record: &Map<String, Value>, plugin_id: &str, browser: bool) -> Result<String> {
    if record.get("installed").and_then(Value::as_bool) != Some(true) {
        return Err(plugin_failure(plugin_id, browser, "not installed"));
    }
    if record.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Err(plugin_failure(plugin_id, browser, "disabled"));
    }
    record
        .get("version")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| plugin_failure(plugin_id, browser, "version invalid"))
}

fn plugin_failure(plugin_id: &str, browser: bool, reason: &str) -> ProError {
    let code = match (browser, reason) {
        (true, "not installed") => DiagnosticCode::BrowserPluginNotInstalled,
        (true, "disabled") => DiagnosticCode::BrowserPluginDisabled,
        (true, _) => DiagnosticCode::BrowserPluginVersionInvalid,
        (false, "not installed") => DiagnosticCode::RemotePluginNotInstalled,
        (false, "disabled") => DiagnosticCode::RemotePluginDisabled,
        (false, _) => DiagnosticCode::RemotePluginVersionInvalid,
    };
    failure(
        DiagnosticStage::PluginInventory,
        code,
        format!("The required plugin is {reason}."),
        "Repair or reinstall the required plugin, then retry !pro.",
        format!("plugin '{plugin_id}' is {reason}"),
    )
}

fn invalid_inventory(detail: String) -> ProError {
    failure(
        DiagnosticStage::PluginInventory,
        DiagnosticCode::PluginInventoryInvalid,
        "Codex returned an invalid installed plugin inventory.",
        "Run `codex plugin list --json`, fix the reported error, then retry !pro.",
        detail,
    )
}

fn failure(
    stage: DiagnosticStage,
    code: DiagnosticCode,
    public_message: impl Into<String>,
    recovery_action: impl Into<String>,
    internal_detail: impl Into<String>,
) -> ProError {
    ProError::Preflight(RuntimeDiagnostic {
        stage,
        code,
        public_message: public_message.into(),
        recovery_action: recovery_action.into(),
        internal_detail: internal_detail.into(),
    })
}
