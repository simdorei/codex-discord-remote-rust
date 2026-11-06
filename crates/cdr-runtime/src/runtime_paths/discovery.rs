use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::PathInputs;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PathDiscoveryError {
    #[error("could not determine the current user home from USERPROFILE or HOME")]
    UserHomeMissing,
    #[error("conflicting Windows environment values for {0}")]
    ConflictingEnvironmentValues(String),
    #[error("cannot observe Codex installation at {path}: {message}")]
    Observation { path: PathBuf, message: String },
}

pub fn discover_inputs(
    environment: &BTreeMap<String, String>,
    root: PathBuf,
) -> Result<PathInputs, PathDiscoveryError> {
    let user_home = first_path(environment, &["USERPROFILE", "HOME"])?
        .ok_or(PathDiscoveryError::UserHomeMissing)?;
    #[cfg(windows)]
    if super::helpers::env_path(environment, "CODEX_EXE")
        .map(super::helpers::clean_executable)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(PathInputs::new(root, user_home, vec![], vec![]));
    }
    let mut local_roots = Vec::new();
    if let Some(local) = first_path(environment, &["LOCALAPPDATA"])? {
        local_roots.push(local.join("OpenAI/Codex/bin"));
    }
    local_roots.push(user_home.join("AppData/Local/OpenAI/Codex/bin"));
    local_roots.push(user_home.join("Library/Application Support/OpenAI/Codex/bin"));
    if cfg!(target_os = "macos") {
        local_roots.push(PathBuf::from(
            "/Applications/Codex.app/Contents/Resources/bin",
        ));
    }
    let mut local_app_candidates = Vec::new();
    for root in &local_roots {
        local_app_candidates.extend(executables_below(root)?);
    }
    let path_candidates = environment_value(environment, "PATH")?
        .into_iter()
        .flat_map(|raw| env::split_paths(raw))
        .map(|directory| directory.join(executable_name()))
        .map(|candidate| observed_file(&candidate).map(|exists| exists.then_some(candidate)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(PathInputs::new(
        root,
        user_home,
        local_app_candidates,
        path_candidates,
    ))
}

#[cfg(windows)]
fn executables_below(root: &Path) -> Result<Vec<PathBuf>, PathDiscoveryError> {
    let direct = root.join(executable_name());
    let mut output = observed_file(&direct)?
        .then_some(direct)
        .into_iter()
        .collect::<Vec<_>>();
    let children = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(output),
        Err(error) => {
            return Err(PathDiscoveryError::Observation {
                path: root.to_owned(),
                message: error.to_string(),
            });
        }
    };
    for child in children {
        let child = child.map_err(|error| PathDiscoveryError::Observation {
            path: root.to_owned(),
            message: error.to_string(),
        })?;
        if !child
            .metadata()
            .map_err(|error| PathDiscoveryError::Observation {
                path: child.path(),
                message: error.to_string(),
            })?
            .is_dir()
        {
            continue;
        }
        let candidate = child.path().join(executable_name());
        if observed_file(&candidate)? {
            output.push(candidate);
        }
    }
    Ok(output)
}

#[cfg(windows)]
fn observed_file(path: &Path) -> Result<bool, PathDiscoveryError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(PathDiscoveryError::Observation {
            path: path.to_owned(),
            message: error.to_string(),
        }),
    }
}

#[cfg(not(windows))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "preserve portable discovery while sharing the Windows interface"
)]
fn observed_file(path: &Path) -> Result<bool, PathDiscoveryError> {
    Ok(path.is_file())
}

#[cfg(not(windows))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "preserve portable discovery while sharing the Windows interface"
)]
fn executables_below(root: &Path) -> Result<Vec<PathBuf>, PathDiscoveryError> {
    let direct = root.join(executable_name());
    let mut output: Vec<_> = direct.is_file().then_some(direct).into_iter().collect();
    for child in fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        let candidate = child.path().join(executable_name());
        if candidate.is_file() {
            output.push(candidate);
        }
    }
    Ok(output)
}

fn environment_value<'a>(
    environment: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<Option<&'a str>, PathDiscoveryError> {
    if let Some(value) = environment.get(name) {
        return Ok(Some(value));
    }
    if !cfg!(windows) {
        return Ok(None);
    }
    let mut matches = environment
        .iter()
        .filter_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.as_str()));
    let value = matches.next();
    if matches.any(|other| Some(other) != value) {
        return Err(PathDiscoveryError::ConflictingEnvironmentValues(
            name.into(),
        ));
    }
    Ok(value)
}

fn first_path(
    environment: &BTreeMap<String, String>,
    names: &[&str],
) -> Result<Option<PathBuf>, PathDiscoveryError> {
    for name in names {
        if let Some(value) = environment_value(environment, name)?
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(Some(PathBuf::from(value)));
        }
    }
    Ok(None)
}

const fn executable_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}
