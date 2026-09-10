use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{PathInputs, PathSource, RuntimePathError};

pub(super) fn resolved_codex_path(
    env: &BTreeMap<String, String>,
    name: &str,
    codex_home: &Path,
    default_name: &str,
    inputs: &PathInputs,
) -> PathBuf {
    env_path(env, name).map_or_else(
        || codex_home.join(default_name),
        |path| expand_home(path, &inputs.user_home),
    )
}

pub(super) fn resolve_executable(
    env: &BTreeMap<String, String>,
    codex_home: &Path,
    inputs: &PathInputs,
) -> Result<(PathBuf, PathSource), RuntimePathError> {
    if let Some(raw) = env_path(env, "CODEX_EXE") {
        let configured = clean_executable(raw);
        if !configured.is_empty() {
            let candidate = expand_home(configured, &inputs.user_home);
            return if is_file(&candidate) {
                Ok((candidate, PathSource::Environment))
            } else {
                Err(RuntimePathError::ConfiguredExecutableMissing(candidate))
            };
        }
    }
    if let Some(candidate) = newest_existing(&inputs.local_app_candidates) {
        return Ok((candidate, PathSource::LocalAppBin));
    }
    let sandbox = codex_home.join(".sandbox-bin").join(executable_name());
    if is_file(&sandbox) {
        return Ok((sandbox, PathSource::SandboxBin));
    }
    let mut saw_windowsapps = false;
    for candidate in &inputs.path_candidates {
        if !is_file(candidate) {
            continue;
        }
        if is_windowsapps(candidate) {
            saw_windowsapps = true;
        } else {
            return Ok((candidate.clone(), PathSource::Path));
        }
    }
    if saw_windowsapps {
        Err(RuntimePathError::WindowsAppsAliasOnly)
    } else {
        Err(RuntimePathError::ExecutableNotFound)
    }
}

pub(super) fn latest_state_db(codex_home: &Path) -> PathBuf {
    let mut candidates = fs::read_dir(codex_home)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            path.is_file()
                && name.starts_with("state_")
                && name.ends_with(".sqlite")
                && !name.ends_with(".sqlite-shm")
                && !name.ends_with(".sqlite-wal")
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| (modified(path), path.file_name().map(ToOwned::to_owned)));
    candidates
        .pop()
        .unwrap_or_else(|| codex_home.join("state_5.sqlite"))
}

fn newest_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    let mut candidates = candidates
        .iter()
        .filter(|path| is_file(path))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| (modified(path), path.clone()));
    candidates.pop()
}

fn modified(path: &Path) -> SystemTime {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(UNIX_EPOCH)
}

pub(super) fn env_path<'a>(env: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    env.get(name)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn clean_executable(raw: &str) -> &str {
    raw.trim()
        .trim_matches(['"', '\''])
        .strip_suffix(",0")
        .unwrap_or(raw.trim().trim_matches(['"', '\'']))
}

pub(super) fn expand_home(path: &str, user_home: &Path) -> PathBuf {
    if path == "~" {
        user_home.to_owned()
    } else if let Some(tail) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        user_home.join(tail)
    } else {
        PathBuf::from(path)
    }
}

fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| metadata.is_file())
}

fn is_windowsapps(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/").to_lowercase();
    normalized.contains("/windowsapps/")
}

const fn executable_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}
