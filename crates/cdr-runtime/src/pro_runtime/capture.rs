use std::path::Path;

use cdr_pro::diagnostics::{DiagnosticCode, DiagnosticStage, RuntimeDiagnostic};
use cdr_pro::fingerprint::{FingerprintError, fingerprint_required_plugins};
use cdr_pro::inventory::read_codex_plugin_inventory;

pub(super) struct CapturedPlugins {
    pub inventory: String,
    pub fingerprint: String,
}

pub(super) async fn capture_plugins(
    codex_exe: &Path,
) -> Result<CapturedPlugins, RuntimeDiagnostic> {
    let inventory = read_codex_plugin_inventory(codex_exe)
        .await
        .map_err(|error| {
            diagnostic(
                DiagnosticStage::PluginInventory,
                DiagnosticCode::PluginInventoryQueryFailed,
                "Codex could not read the installed plugin inventory.",
                "Run `codex plugin list --json`, fix the reported error, then retry !pro.",
                error.to_string(),
            )
        })?;
    let fingerprint_input = inventory.clone();
    let fingerprint =
        tokio::task::spawn_blocking(move || fingerprint_required_plugins(&fingerprint_input))
            .await
            .map_err(|error| content_diagnostic(&error.to_string()))?
            .map_err(|error| fingerprint_diagnostic(&error))?;
    Ok(CapturedPlugins {
        inventory,
        fingerprint,
    })
}

fn fingerprint_diagnostic(error: &FingerprintError) -> RuntimeDiagnostic {
    match error {
        FingerprintError::Inventory(_) => diagnostic(
            DiagnosticStage::PluginInventory,
            DiagnosticCode::PluginInventoryInvalid,
            "Codex returned invalid plugin source metadata.",
            "Repair or reinstall the required plugins, then retry !pro.",
            error.to_string(),
        ),
        FingerprintError::Content(_) | FingerprintError::Io(_) => {
            content_diagnostic(&error.to_string())
        }
    }
}

fn content_diagnostic(detail: &str) -> RuntimeDiagnostic {
    diagnostic(
        DiagnosticStage::PluginContent,
        DiagnosticCode::PluginContentUnverified,
        "The installed plugin files could not be verified; Chrome availability was not tested.",
        "Repair or reinstall the required plugins, restart the remote bot, then retry !pro.",
        detail.to_owned(),
    )
}

fn diagnostic(
    stage: DiagnosticStage,
    code: DiagnosticCode,
    public_message: impl Into<String>,
    recovery_action: impl Into<String>,
    internal_detail: impl Into<String>,
) -> RuntimeDiagnostic {
    RuntimeDiagnostic {
        stage,
        code,
        public_message: public_message.into(),
        recovery_action: recovery_action.into(),
        internal_detail: internal_detail.into(),
    }
}
