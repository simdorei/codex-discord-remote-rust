use std::{fs, path::Path};

use serde_json::Value;

use crate::diagnostics::{DiagnosticCode, DiagnosticStage, Result};

use super::failure;

pub fn expected_remote_plugin_version(path: &Path) -> Result<String> {
    let raw = fs::read_to_string(path).map_err(|error| unavailable(&error))?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let manifest: Value = serde_json::from_str(raw).map_err(|error| unavailable(&error))?;
    let object = manifest.as_object().ok_or_else(|| {
        failure(
            DiagnosticStage::PluginManifest,
            DiagnosticCode::RemoteManifestInvalid,
            "The remote plugin manifest is invalid.",
            "Reinstall the remote plugin, then retry !pro.",
            "remote plugin manifest must be a JSON object",
        )
    })?;
    object
        .get("version")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            failure(
                DiagnosticStage::PluginManifest,
                DiagnosticCode::RemoteManifestInvalid,
                "The remote plugin manifest has no valid version.",
                "Reinstall the remote plugin, then retry !pro.",
                "remote plugin manifest.version must be a non-empty string",
            )
        })
}

fn unavailable(detail: &impl std::fmt::Display) -> crate::diagnostics::ProError {
    failure(
        DiagnosticStage::PluginManifest,
        DiagnosticCode::RemoteManifestUnavailable,
        "The remote plugin manifest could not be read.",
        "Reinstall the remote plugin, then retry !pro.",
        format!("remote plugin manifest is unavailable: {detail}"),
    )
}
