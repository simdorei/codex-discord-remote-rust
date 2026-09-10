use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticStage {
    PluginInventory,
    PluginManifest,
    PluginContent,
    ResidentAppServer,
    RemoteMcp,
    ProjectTicket,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticCode {
    PluginInventoryQueryFailed,
    PluginInventoryInvalid,
    RemotePluginMissing,
    RemotePluginNotInstalled,
    RemotePluginDisabled,
    RemotePluginVersionInvalid,
    RemotePluginVersionMismatch,
    BrowserPluginMissing,
    BrowserPluginNotInstalled,
    BrowserPluginDisabled,
    BrowserPluginVersionInvalid,
    PluginContentUnverified,
    RemoteManifestUnavailable,
    RemoteManifestInvalid,
    ResidentUnhealthy,
    ResidentSnapshotFailed,
    ResidentSnapshotMissing,
    ResidentStale,
    RemoteMcpConfigurationInvalid,
    RemoteMcpConnectionFailed,
    RemoteMcpNotConfigured,
    ProjectTicketTimezoneInvalid,
    ProjectTicketExpired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDiagnostic {
    pub stage: DiagnosticStage,
    pub code: DiagnosticCode,
    pub public_message: String,
    pub recovery_action: String,
    pub internal_detail: String,
}

#[derive(Debug, Error)]
pub enum ProError {
    #[error("{}", .0.internal_detail)]
    Preflight(RuntimeDiagnostic),
}

impl ProError {
    #[must_use]
    pub const fn diagnostic(&self) -> Option<&RuntimeDiagnostic> {
        match self {
            Self::Preflight(diagnostic) => Some(diagnostic),
        }
    }
}

pub type Result<T> = std::result::Result<T, ProError>;
