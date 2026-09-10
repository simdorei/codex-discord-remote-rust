use std::collections::BTreeMap;
use std::path::PathBuf;

use thiserror::Error;

mod discovery;
mod helpers;

pub use discovery::{PathDiscoveryError, discover_inputs};
use helpers::{env_path, expand_home, latest_state_db, resolve_executable, resolved_codex_path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathSource {
    Environment,
    SandboxBin,
    LocalAppBin,
    Path,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
}

#[derive(Clone, Debug)]
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
    #[error("configured CODEX_EXE does not exist or is not a file: {0}")]
    ConfiguredExecutableMissing(PathBuf),
    #[error("Codex resolves only to a WindowsApps alias; configure the real CODEX_EXE")]
    WindowsAppsAliasOnly,
    #[error("no usable Codex executable was found")]
    ExecutableNotFound,
}

impl RuntimePaths {
    pub fn resolve(
        env: &BTreeMap<String, String>,
        inputs: &PathInputs,
    ) -> Result<Self, RuntimePathError> {
        let codex_home = env_path(env, "CODEX_HOME").map_or_else(
            || inputs.user_home.join(".codex"),
            |path| expand_home(path, &inputs.user_home),
        );
        let root = env_path(env, "CODEX_DISCORD_ROOT").map_or_else(
            || inputs.root.clone(),
            |path| expand_home(path, &inputs.user_home),
        );
        let mirror_db = env_path(env, "CODEX_DISCORD_MIRROR_DB").map_or_else(
            || root.join("discord_mirror.sqlite"),
            |path| expand_home(path, &inputs.user_home),
        );
        let state_db = env_path(env, "CODEX_STATE_DB").map_or_else(
            || latest_state_db(&codex_home),
            |path| expand_home(path, &inputs.user_home),
        );
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
        let (codex_exe, codex_exe_source) = resolve_executable(env, &codex_home, inputs)?;
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
        })
    }
}
