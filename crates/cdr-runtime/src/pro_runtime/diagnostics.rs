use std::fmt::Write as _;

use cdr_app_server::AppServerError;
use cdr_pro::diagnostics::{DiagnosticCode, DiagnosticStage, ProError, RuntimeDiagnostic};

use crate::prompt_preprocessor::PromptPreprocessError;

pub(super) fn public_error(error: &ProError) -> PromptPreprocessError {
    let diagnostic = error
        .diagnostic()
        .expect("all Pro errors carry a runtime diagnostic");
    eprintln!(
        "pro_preflight_failed stage={:?} code={:?} detail={}",
        diagnostic.stage,
        diagnostic.code,
        diagnostic
            .internal_detail
            .chars()
            .take(500)
            .collect::<String>()
    );
    PromptPreprocessError {
        public_message: diagnostic.public_message.clone(),
        recovery_action: diagnostic.recovery_action.clone(),
    }
}

pub(super) fn not_configured() -> ProError {
    failure(
        DiagnosticCode::RemoteMcpNotConfigured,
        "The local PC connection is not configured.",
        "Configure remote MCP, restart the remote bot, then retry !pro.",
        "remote MCP is not configured",
    )
}

pub(super) fn connection_failure() -> ProError {
    failure(
        DiagnosticCode::RemoteMcpConnectionFailed,
        "The local PC connection did not become ready.",
        "Restart the remote bot, verify remote MCP connectivity, then retry !pro.",
        "remote MCP gateway hello has not been acknowledged",
    )
}

pub(super) fn restart_failure(mut error: ProError, source: &AppServerError) -> ProError {
    let ProError::Preflight(diagnostic) = &mut error;
    let _ = write!(
        diagnostic.internal_detail,
        "; automatic resident refresh failed error={source}"
    );
    error
}

fn failure(
    code: DiagnosticCode,
    public_message: &str,
    recovery_action: &str,
    internal_detail: &str,
) -> ProError {
    ProError::Preflight(RuntimeDiagnostic {
        stage: DiagnosticStage::RemoteMcp,
        code,
        public_message: public_message.into(),
        recovery_action: recovery_action.into(),
        internal_detail: internal_detail.into(),
    })
}
