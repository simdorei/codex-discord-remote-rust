use super::{
    RuntimePathError,
    helpers::{env_path, expand_home, latest_state_db},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Offline readers and the running bot must resolve the same Codex state DB.
pub(crate) fn resolve(
    environment: &BTreeMap<String, String>,
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
    let home = env_path(environment, "CODEX_HOME")
        .map(expand)
        .transpose()?
        .or_else(|| user_home.map(|path| path.join(".codex")));
    let state = env_path(environment, "CODEX_STATE_DB")
        .map(expand)
        .transpose()?;
    match (home, state) {
        (Some(home), Some(state)) => Ok((home, state)),
        (Some(home), None) => {
            let state = latest_state_db(&home);
            Ok((home, state))
        }
        (None, Some(state)) => Ok((PathBuf::new(), state)),
        (None, None) => Err(RuntimePathError::UserHomeMissing),
    }
}
