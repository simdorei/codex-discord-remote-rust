use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use super::{
    RuntimePathError,
    helpers::{env_path, expand_home},
};

/// Keep offline backup and normal startup on the exact same configured database.
pub(crate) fn resolve(
    environment: &BTreeMap<String, String>,
    default_root: &Path,
    user_home: Option<&Path>,
) -> Result<(PathBuf, PathBuf), RuntimePathError> {
    let expand = |raw: &str| -> Result<PathBuf, RuntimePathError> {
        if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
            Ok(expand_home(
                raw,
                user_home.ok_or(RuntimePathError::UserHomeMissing)?,
            ))
        } else {
            Ok(PathBuf::from(raw))
        }
    };
    let root = env_path(environment, "CODEX_DISCORD_ROOT")
        .map(expand)
        .transpose()?
        .unwrap_or_else(|| default_root.to_path_buf());
    let database = env_path(environment, "CODEX_DISCORD_MIRROR_DB")
        .map(expand)
        .transpose()?
        .unwrap_or_else(|| root.join("discord_mirror.sqlite"));
    Ok((root, database))
}
