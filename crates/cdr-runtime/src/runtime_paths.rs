use std::collections::BTreeMap;
use std::path::PathBuf;

use thiserror::Error;

#[cfg(windows)]
mod artifact;
mod discovery;
mod helpers;
#[cfg(windows)]
pub mod receipt;
#[cfg(windows)]
pub mod selection;
#[cfg(windows)]
pub mod selection_worker;
pub(crate) mod state_paths;
pub(crate) mod store_paths;
#[cfg(windows)]
mod version;

pub use discovery::{PathDiscoveryError, discover_inputs};
#[cfg(not(windows))]
use helpers::resolve_executable;
use helpers::{env_path, expand_home, resolved_codex_path};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PathSource {
    Environment,
    SandboxBin,
    LocalAppBin,
    Path,
    StorePackage,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RuntimePaths {
    pub root: PathBuf,
    pub codex_home: PathBuf,
    pub mirror_db: PathBuf,
    pub state_db: PathBuf,
    pub bridge_state: PathBuf,
    pub log_db: PathBuf,
    pub global_state: PathBuf,
    pub session_index: PathBuf,
    pub archived_sessions: PathBuf,
    pub maintenance_backup_root: PathBuf,
    pub attachment_dir: PathBuf,
    pub codex_exe: PathBuf,
    pub codex_exe_source: PathSource,
    #[cfg(windows)]
    pub codex_selection: Option<selection::CodexSelection>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PathInputs {
    pub root: PathBuf,
    pub user_home: PathBuf,
    pub local_app_candidates: Vec<PathBuf>,
    pub path_candidates: Vec<PathBuf>,
}

impl PathInputs {
    #[must_use]
    pub const fn new(
        root: PathBuf,
        user_home: PathBuf,
        local_app_candidates: Vec<PathBuf>,
        path_candidates: Vec<PathBuf>,
    ) -> Self {
        Self {
            root,
            user_home,
            local_app_candidates,
            path_candidates,
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RuntimePathError {
    #[error("could not expand configured database path: user home is missing")]
    UserHomeMissing,
    #[error("configured CODEX_EXE does not exist or is not a file: {0}")]
    ConfiguredExecutableMissing(PathBuf),
    #[error(
        "Codex installation is incomplete: missing {0}; selection held; set CODEX_EXE to codex.exe in a complete Codex installation containing its matching codex-code-mode-host.exe"
    )]
    CodeModeHostMissing(PathBuf),
    #[error("Codex resolves only to a WindowsApps alias; configure the real CODEX_EXE")]
    WindowsAppsAliasOnly,
    #[error("no usable Codex executable was found")]
    ExecutableNotFound,
    #[error("Codex selection held: {0}")]
    SelectionFailed(String),
}

impl RuntimePaths {
    /// Offline store maintenance must not discover or execute a Codex installation.
    pub fn store_database(
        environment: &BTreeMap<String, String>,
        root: &std::path::Path,
    ) -> Result<PathBuf, RuntimePathError> {
        let home = ["USERPROFILE", "HOME"]
            .into_iter()
            .find_map(|name| env_path(environment, name))
            .map(std::path::Path::new);
        store_paths::resolve(environment, root, home).map(|(_, database)| database)
    }

    pub fn discover(
        environment: &BTreeMap<String, String>,
        root: &std::path::Path,
        deadline: std::time::Instant,
        cleanup: std::time::Duration,
    ) -> Result<Self, RuntimePathError> {
        #[cfg(windows)]
        {
            selection_worker::discover(environment, root, deadline, cleanup)
        }
        #[cfg(not(windows))]
        {
            let _ = cleanup;
            let inputs = discover_inputs(environment, root.to_owned())
                .map_err(|e| RuntimePathError::SelectionFailed(e.to_string()))?;
            Self::resolve_before(environment, &inputs, deadline)
        }
    }

    pub fn app_server_config(
        &self,
        deadline: std::time::Instant,
    ) -> Result<cdr_app_server::AppServerConfig, RuntimePathError> {
        let mut config =
            cdr_app_server::AppServerConfig::new(&self.codex_exe).with_environment([(
                "CODEX_HOME".into(),
                self.codex_home.to_string_lossy().into_owned(),
            )]);
        config.startup_deadline = Some(deadline);
        #[cfg(windows)]
        if let Some(selection) = &self.codex_selection {
            let work_end = deadline
                .checked_sub(std::time::Duration::from_secs(8))
                .ok_or_else(|| {
                    RuntimePathError::SelectionFailed("startup deadline exhausted".into())
                })?;
            let runtime = std::env::current_exe()
                .map_err(|e| RuntimePathError::SelectionFailed(e.to_string()))?;
            config.retained_executable_files =
                selection_worker::lease_using(&runtime, selection, &self.root, work_end)?;
        }
        Ok(config)
    }

    pub fn resolve(
        env: &BTreeMap<String, String>,
        inputs: &PathInputs,
    ) -> Result<Self, RuntimePathError> {
        Self::resolve_before(
            env,
            inputs,
            std::time::Instant::now() + std::time::Duration::from_secs(15),
        )
    }

    pub fn resolve_before(
        env: &BTreeMap<String, String>,
        inputs: &PathInputs,
        deadline: std::time::Instant,
    ) -> Result<Self, RuntimePathError> {
        let (codex_home, state_db) = state_paths::resolve(env, Some(&inputs.user_home))?;
        let (root, mirror_db) = store_paths::resolve(env, &inputs.root, Some(&inputs.user_home))?;
        let bridge_state = env_path(env, "CODEX_BRIDGE_STATE").map_or_else(
            || codex_home.join("codex_desktop_bridge_state.json"),
            |path| expand_home(path, &inputs.user_home),
        );
        let log_db = resolved_codex_path(env, "CODEX_LOG_DB", &codex_home, "logs_2.sqlite", inputs);
        let global_state = resolved_codex_path(
            env,
            "CODEX_GLOBAL_STATE",
            &codex_home,
            ".codex-global-state.json",
            inputs,
        );
        let session_index = resolved_codex_path(
            env,
            "CODEX_SESSION_INDEX",
            &codex_home,
            "session_index.jsonl",
            inputs,
        );
        let archived_sessions = resolved_codex_path(
            env,
            "CODEX_ARCHIVED_SESSIONS_DIR",
            &codex_home,
            "archived_sessions",
            inputs,
        );
        let maintenance_backup_root = resolved_codex_path(
            env,
            "CODEX_MAINTENANCE_BACKUP_ROOT",
            &codex_home,
            "maintenance_backups",
            inputs,
        );
        let attachment_dir = env_path(env, "DISCORD_ATTACHMENT_DOWNLOAD_DIR").map_or_else(
            || root.join(".codex-discord-attachments"),
            |path| expand_home(path, &inputs.user_home),
        );
        #[cfg(windows)]
        let codex_selection = selection::select(env, &codex_home, inputs, deadline)?;
        #[cfg(windows)]
        let (codex_exe, codex_exe_source) = (codex_selection.path.clone(), codex_selection.source);
        #[cfg(not(windows))]
        let (codex_exe, codex_exe_source) = {
            let _ = deadline;
            resolve_executable(env, &codex_home, inputs)?
        };
        Ok(Self {
            root,
            codex_home,
            mirror_db,
            state_db,
            bridge_state,
            log_db,
            global_state,
            session_index,
            archived_sessions,
            maintenance_backup_root,
            attachment_dir,
            codex_exe,
            codex_exe_source,
            #[cfg(windows)]
            codex_selection: Some(codex_selection),
        })
    }
}
