use std::fs::File;
use std::io::Read;
use std::path::{Component, Path};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use walkdir::WalkDir;

use crate::preflight::{CHROME_PLUGIN_ID, REMOTE_PLUGIN_ID};

#[derive(Debug, Error)]
pub enum FingerprintError {
    #[error("invalid plugin inventory: {0}")]
    Inventory(String),
    #[error("plugin content could not be verified: {0}")]
    Content(String),
    #[error("plugin content I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, FingerprintError>;

#[derive(Serialize)]
struct PluginEvidence {
    plugin_id: String,
    version: String,
    source_path: String,
    tree_sha256: String,
}

pub fn fingerprint_required_plugins(inventory_json: &str) -> Result<String> {
    let inventory: Value = serde_json::from_str(inventory_json)
        .map_err(|error| FingerprintError::Inventory(error.to_string()))?;
    let records = inventory
        .as_object()
        .and_then(|object| object.get("installed"))
        .and_then(Value::as_array)
        .ok_or_else(|| FingerprintError::Inventory("installed must be an array".into()))?;
    let evidence = [REMOTE_PLUGIN_ID, CHROME_PLUGIN_ID]
        .into_iter()
        .map(|plugin_id| plugin_evidence(records, plugin_id))
        .collect::<Result<Vec<_>>>()?;
    let canonical = serde_json::to_vec(&evidence)
        .map_err(|error| FingerprintError::Inventory(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

fn plugin_evidence(records: &[Value], plugin_id: &str) -> Result<PluginEvidence> {
    let matches = records
        .iter()
        .filter_map(Value::as_object)
        .filter(|record| record.get("pluginId").and_then(Value::as_str) == Some(plugin_id))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(FingerprintError::Inventory(format!(
            "plugin '{plugin_id}' was not installed exactly once"
        )));
    }
    let record = matches[0];
    if record.get("installed").and_then(Value::as_bool) != Some(true)
        || record.get("enabled").and_then(Value::as_bool) != Some(true)
    {
        return Err(FingerprintError::Inventory(format!(
            "plugin '{plugin_id}' is not installed and enabled"
        )));
    }
    let version = record
        .get("version")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| FingerprintError::Inventory(format!("plugin '{plugin_id}' version")))?;
    let raw_path = record
        .get("source")
        .and_then(Value::as_object)
        .and_then(|source| source.get("path"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| FingerprintError::Inventory(format!("plugin '{plugin_id}' source.path")))?;
    let root = Path::new(raw_path).canonicalize().map_err(|error| {
        FingerprintError::Content(format!("plugin '{plugin_id}' source unavailable: {error}"))
    })?;
    if !root.is_dir() {
        return Err(FingerprintError::Content(format!(
            "plugin '{plugin_id}' source is not a directory"
        )));
    }
    Ok(PluginEvidence {
        plugin_id: plugin_id.into(),
        version: version.into(),
        source_path: normalized_path(&root),
        tree_sha256: tree_digest(&root)?,
    })
}

pub fn tree_digest(root: &Path) -> Result<String> {
    let root = root.canonicalize()?;
    let mut digest = Sha256::new();
    for entry in WalkDir::new(&root).follow_links(false).sort_by_file_name() {
        let entry = entry.map_err(|error| {
            FingerprintError::Content(format!("plugin source tree walk failed: {error}"))
        })?;
        if entry.path() == root {
            continue;
        }
        let relative = entry.path().strip_prefix(&root).map_err(|error| {
            FingerprintError::Content(format!("plugin path escaped root: {error}"))
        })?;
        if ignored(relative) {
            if entry.file_type().is_dir() {
                continue;
            }
            continue;
        }
        reject_link_or_escape(&root, entry.path(), relative)?;
        if entry.file_type().is_file() {
            hash_file(&mut digest, relative, entry.path())?;
        }
    }
    Ok(hex::encode(digest.finalize()))
}

fn ignored(relative: &Path) -> bool {
    relative
        .components()
        .any(|part| matches!(part, Component::Normal(name) if name == "__pycache__"))
        || relative.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("pyc") || extension.eq_ignore_ascii_case("pyo")
        })
}

fn reject_link_or_escape(root: &Path, path: &Path, relative: &Path) -> Result<()> {
    let metadata = path.symlink_metadata()?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(FingerprintError::Content(format!(
            "plugin source contains a link: {}",
            relative.display()
        )));
    }
    let resolved = path.canonicalize()?;
    if !resolved.starts_with(root) {
        return Err(FingerprintError::Content(format!(
            "plugin source escapes root: {}",
            relative.display()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
const fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn hash_file(digest: &mut Sha256, relative: &Path, path: &Path) -> Result<()> {
    let relative = relative.to_string_lossy().replace('\\', "/");
    digest.update(relative.len().to_be_bytes());
    digest.update(relative.as_bytes());
    digest.update(path.metadata()?.len().to_be_bytes());
    let mut file = File::open(path)?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(())
}

fn normalized_path(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.to_string_lossy().replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().into_owned()
    }
}
