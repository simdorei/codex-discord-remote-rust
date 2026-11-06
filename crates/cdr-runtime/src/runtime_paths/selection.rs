use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::helpers::{clean_executable, env_path, expand_home};
use super::version::Version;
use super::{PathInputs, PathSource, RuntimePathError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CandidateObservation {
    pub path: PathBuf,
    pub source: PathSource,
    pub version: Option<String>,
    pub rejection: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodexSelection {
    pub schema_version: u32,
    pub path: PathBuf,
    pub source: PathSource,
    pub version: String,
    pub candidates: Vec<CandidateObservation>,
    pub artifacts: Vec<super::artifact::ArtifactFile>,
}

impl CodexSelection {
    pub fn lease(
        &self,
        deadline: Instant,
    ) -> Result<Vec<std::sync::Arc<std::fs::File>>, RuntimePathError> {
        super::artifact::lease(&self.artifacts, deadline)
    }
}

pub(super) fn select(
    environment: &BTreeMap<String, String>,
    home: &Path,
    inputs: &PathInputs,
    deadline: Instant,
) -> Result<CodexSelection, RuntimePathError> {
    if let Some(raw) = env_path(environment, "CODEX_EXE")
        .map(clean_executable)
        .filter(|v| !v.is_empty())
    {
        let configured = expand_home(raw, &inputs.user_home);
        let path = if configured.is_absolute() {
            configured
        } else {
            inputs.root.join(configured)
        };
        return pinned(path, deadline);
    }
    automatic(home, inputs, deadline)
}

fn pinned(path: PathBuf, deadline: Instant) -> Result<CodexSelection, RuntimePathError> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "cmd" | "bat" | "ps1"))
    {
        return Err(failed(
            "script launcher cannot certify executable identity; set CODEX_EXE to the native codex.exe",
        ));
    }
    match fs::metadata(&path) {
        Ok(meta) if meta.is_file() => {}
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(failed(format!("pin not observable: {error}")));
        }
        _ => return Err(RuntimePathError::ConfiguredExecutableMissing(path)),
    }
    complete_bundle(&path)?;
    let version_lease = super::artifact::locked_file(&path)?;
    let require_host = requires_host(&path);
    let path = cdr_windows_native::resolved_file_path(&version_lease)
        .map_err(|e| failed(e.to_string()))?;
    let require_host = require_host || requires_host(&path);
    complete_bundle_at(&path, require_host)?;
    let version = probe_version(&path, deadline)?;
    let artifacts = super::artifact::observe(&path, require_host, deadline)?;
    Ok(CodexSelection {
        schema_version: 1,
        path,
        source: PathSource::Environment,
        version,
        candidates: vec![],
        artifacts,
    })
}

fn automatic(
    home: &Path,
    inputs: &PathInputs,
    deadline: Instant,
) -> Result<CodexSelection, RuntimePathError> {
    let mut candidates: Vec<_> = inputs
        .local_app_candidates
        .iter()
        .map(|p| (p.clone(), source(p, PathSource::LocalAppBin)))
        .collect();
    let sandbox = home.join(".sandbox-bin/codex.exe");
    match fs::metadata(&sandbox) {
        Ok(meta) if meta.is_file() => candidates.push((sandbox, PathSource::SandboxBin)),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(failed(format!(
                "cannot observe {}: {error}",
                sandbox.display()
            )));
        }
        _ => {}
    }
    candidates.extend(
        inputs
            .path_candidates
            .iter()
            .map(|p| (p.clone(), source(p, PathSource::Path))),
    );
    let mut observations = Vec::new();
    let mut ranked = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut version_leases = Vec::new();
    let mut alias = false;
    for (path, source) in candidates {
        if !seen.insert(normalize(&path)) {
            continue;
        }
        if windows_alias(&path) {
            alias = true;
            continue;
        }
        let require_host = requires_host(&path);
        let version_lease = super::artifact::locked_file(&path)?;
        let path = cdr_windows_native::resolved_file_path(&version_lease)
            .map_err(|e| failed(e.to_string()))?;
        let require_host = require_host || requires_host(&path);
        version_leases.push(version_lease);
        let version = probe_version(&path, deadline)?;
        let parsed = Version::parse(&version).ok_or_else(|| failed("invalid semantic version"))?;
        let rejection = complete_bundle_at(&path, require_host)
            .err()
            .map(|e| e.to_string());
        observations.push(CandidateObservation {
            path: path.clone(),
            source,
            version: Some(version.clone()),
            rejection: rejection.clone(),
        });
        ranked.push((parsed, path, source, version, rejection, require_host));
    }
    ranked.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    let Some(highest) = ranked.first().map(|v| v.0.clone()) else {
        return Err(if alias {
            RuntimePathError::WindowsAppsAliasOnly
        } else {
            RuntimePathError::ExecutableNotFound
        });
    };
    let chosen = ranked
        .into_iter()
        .find(|entry| entry.0 == highest && entry.4.is_none());
    let Some((_, path, source, version, _, require_host)) = chosen else {
        return Err(failed(format!(
            "highest installed Codex version is incomplete; automatic downgrade held: {}",
            serde_json::to_string(&observations).unwrap_or_default()
        )));
    };
    let artifacts = super::artifact::observe(&path, require_host, deadline)?;
    Ok(CodexSelection {
        schema_version: 1,
        path,
        source,
        version,
        candidates: observations,
        artifacts,
    })
}

fn normalize(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

pub(super) fn source(path: &Path, otherwise: PathSource) -> PathSource {
    if normalize(path).contains("/windowsapps/openai.codex_") {
        PathSource::StorePackage
    } else {
        otherwise
    }
}

fn windows_alias(path: &Path) -> bool {
    normalize(path).contains("/windowsapps/")
        && source(path, PathSource::Path) != PathSource::StorePackage
}

pub(super) fn complete_bundle(path: &Path) -> Result<(), RuntimePathError> {
    complete_bundle_at(path, requires_host(path))
}

fn complete_bundle_at(path: &Path, require_host: bool) -> Result<(), RuntimePathError> {
    if require_host || requires_host(path) {
        let host = path.with_file_name("codex-code-mode-host.exe");
        match fs::metadata(&host) {
            Ok(meta) if meta.is_file() => {}
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(failed(format!(
                    "cannot observe {}: {error}",
                    host.display()
                )));
            }
            _ => return Err(RuntimePathError::CodeModeHostMissing(host)),
        }
    }
    Ok(())
}

pub(super) fn requires_host(path: &Path) -> bool {
    let normalized = normalize(path);
    normalized.contains("/openai/codex/bin/")
        || normalized.contains("/.sandbox-bin/")
        || normalized.contains("/windowsapps/openai.codex_")
}

pub fn probe_version(path: &Path, deadline: Instant) -> Result<String, RuntimePathError> {
    let lease = super::artifact::locked_file(path)?;
    let path = cdr_windows_native::resolved_file_path(&lease).map_err(|e| failed(e.to_string()))?;
    let deadline = deadline.min(Instant::now() + Duration::from_secs(2));
    let environment: HashMap<_, _> = std::env::vars().collect();
    let output = cdr_windows_native::capture_bounded(
        &path,
        &["--version".into()],
        path.parent().unwrap_or(Path::new(".")),
        &environment,
        deadline,
        Duration::from_millis(500),
        65_536,
    )
    .map_err(|error| failed(format!("{}: {error}", path.display())))?;
    if output.code != 0 {
        return Err(failed(format!(
            "{}: version probe exited {}",
            path.display(),
            output.code
        )));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| failed("version output is not UTF-8"))?
        .trim();
    let version = text
        .strip_prefix("codex-cli ")
        .filter(|raw| Version::parse(raw).is_some())
        .ok_or_else(|| failed(format!("{}: version not observable", path.display())))?;
    Ok(version.to_owned())
}

pub(super) fn failed(message: impl Into<String>) -> RuntimePathError {
    RuntimePathError::SelectionFailed(message.into())
}
