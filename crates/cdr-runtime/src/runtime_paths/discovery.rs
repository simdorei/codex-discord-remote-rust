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
}

pub fn discover_inputs(
    environment: &BTreeMap<String, String>,
    root: PathBuf,
) -> Result<PathInputs, PathDiscoveryError> {
    let user_home = first_path(environment, &["USERPROFILE", "HOME"])
        .ok_or(PathDiscoveryError::UserHomeMissing)?;
    let mut local_roots = Vec::new();
    if let Some(local) = first_path(environment, &["LOCALAPPDATA"]) {
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
    let path_candidates = environment
        .get("PATH")
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

fn first_path(environment: &BTreeMap<String, String>, names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|name| {
        environment
            .get(*name)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

const fn executable_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}
