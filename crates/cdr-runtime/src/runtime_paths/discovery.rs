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
}

pub fn discover_inputs(
    environment: &BTreeMap<String, String>,
    root: PathBuf,
) -> Result<PathInputs, PathDiscoveryError> {
    let user_home = first_path(environment, &["USERPROFILE", "HOME"])?
        .ok_or(PathDiscoveryError::UserHomeMissing)?;
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
    let local_app_candidates = local_roots
        .iter()
        .flat_map(|root| executables_below(root))
        .collect();
    let path_candidates = environment_value(environment, "PATH")?
        .into_iter()
        .flat_map(|raw| env::split_paths(raw))
        .map(|directory| directory.join(executable_name()))
        .filter(|candidate| candidate.is_file())
        .collect();
    Ok(PathInputs::new(
        root,
        user_home,
        local_app_candidates,
        path_candidates,
    ))
}

fn executables_below(root: &Path) -> Vec<PathBuf> {
    let direct = root.join(executable_name());
    let mut output = direct
        .is_file()
        .then_some(direct)
        .into_iter()
        .collect::<Vec<_>>();
    let children = fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok);
    for child in children {
        let candidate = child.path().join(executable_name());
        if candidate.is_file() {
            output.push(candidate);
        }
    }
    output
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
