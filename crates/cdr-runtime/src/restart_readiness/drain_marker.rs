use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::drain::{DrainFenceKey, DrainGateError};

pub(crate) const IDENTITY_NAME: &str = ".codex_discord_rust.drain.identity";
pub(crate) const PREPARE_NAME: &str = ".codex_discord_rust.drain.prepare";
pub(crate) const ACK_NAME: &str = ".codex_discord_rust.drain.ack";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeMarker {
    pub runtime_id: String,
    pub process_id: u32,
}

#[derive(Debug, Error)]
pub enum DrainMarkerError {
    #[error("could not access restart drain marker {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("restart drain marker {path} is malformed: {reason}")]
    Malformed { path: PathBuf, reason: String },
    #[error(transparent)]
    InvalidKey(#[from] DrainGateError),
}

pub(crate) fn identity_path(root: &Path) -> PathBuf {
    root.join(IDENTITY_NAME)
}

pub(crate) fn prepare_path(root: &Path) -> PathBuf {
    root.join(PREPARE_NAME)
}

pub(crate) fn ack_path(root: &Path) -> PathBuf {
    root.join(ACK_NAME)
}

pub(crate) fn publish_identity(
    root: &Path,
    marker: &RuntimeMarker,
) -> Result<(), DrainMarkerError> {
    atomic_write(
        &identity_path(root),
        &format!(
            "version=1\nruntime_id={}\npid={}\nstate=open\n",
            marker.runtime_id, marker.process_id
        ),
    )
}

pub(crate) fn publish_ack(root: &Path, key: &DrainFenceKey) -> Result<(), DrainMarkerError> {
    atomic_write(
        &ack_path(root),
        &format!(
            "version=1\nruntime_id={}\nprocess_identity={}\nnonce={}\nstate=sealed\n",
            key.runtime_id(),
            key.process_identity(),
            key.nonce()
        ),
    )
}

pub(crate) fn read_identity(root: &Path) -> Result<Option<RuntimeMarker>, DrainMarkerError> {
    let path = identity_path(root);
    let Some(fields) = read_fields(&path)? else {
        return Ok(None);
    };
    require(&path, &fields, "version", "1")?;
    require(&path, &fields, "state", "open")?;
    let runtime_id = field(&path, &fields, "runtime_id")?.to_owned();
    let process_id = field(&path, &fields, "pid")?
        .parse()
        .map_err(|_| malformed(&path, "pid must be an unsigned integer"))?;
    Ok(Some(RuntimeMarker {
        runtime_id,
        process_id,
    }))
}

pub(crate) fn read_prepare(root: &Path) -> Result<Option<DrainFenceKey>, DrainMarkerError> {
    read_fence(&prepare_path(root), None)
}

pub(crate) fn read_ack(root: &Path) -> Result<Option<DrainFenceKey>, DrainMarkerError> {
    read_fence(&ack_path(root), Some("sealed"))
}

pub(crate) fn read_restart(root: &Path) -> Result<Option<DrainFenceKey>, DrainMarkerError> {
    read_fence(&root.join(".codex_discord_rust.restart"), None)
}

pub(crate) fn remove_identity_if_owned(
    root: &Path,
    expected: &RuntimeMarker,
) -> Result<(), DrainMarkerError> {
    if read_identity(root)?.as_ref() == Some(expected) {
        remove_file(&identity_path(root))?;
    }
    Ok(())
}

pub(crate) fn remove_ack_if_owned(
    root: &Path,
    expected: &DrainFenceKey,
) -> Result<(), DrainMarkerError> {
    if read_ack(root)?.as_ref() == Some(expected) {
        remove_file(&ack_path(root))?;
    }
    Ok(())
}

fn read_fence(
    path: &Path,
    expected_state: Option<&str>,
) -> Result<Option<DrainFenceKey>, DrainMarkerError> {
    let Some(fields) = read_fields(path)? else {
        return Ok(None);
    };
    require(path, &fields, "version", "1")?;
    if let Some(state) = expected_state {
        require(path, &fields, "state", state)?;
    }
    Ok(Some(DrainFenceKey::new(
        field(path, &fields, "runtime_id")?,
        field(path, &fields, "process_identity")?,
        field(path, &fields, "nonce")?,
    )?))
}

fn read_fields(path: &Path) -> Result<Option<BTreeMap<String, String>>, DrainMarkerError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error(path, source)),
    };
    if text.len() > 4_096 {
        return Err(malformed(path, "marker exceeds 4096 bytes"));
    }
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| malformed(path, "line does not contain '='"))?;
        if name.is_empty() || value.is_empty() || fields.insert(name.into(), value.into()).is_some()
        {
            return Err(malformed(path, "field is empty or duplicated"));
        }
    }
    Ok(Some(fields))
}

fn field<'a>(
    path: &Path,
    fields: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str, DrainMarkerError> {
    fields
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| malformed(path, &format!("missing {name}")))
}

fn require(
    path: &Path,
    fields: &BTreeMap<String, String>,
    name: &str,
    expected: &str,
) -> Result<(), DrainMarkerError> {
    if field(path, fields, name)? != expected {
        return Err(malformed(path, &format!("invalid {name}")));
    }
    Ok(())
}

fn atomic_write(path: &Path, text: &str) -> Result<(), DrainMarkerError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| io_error(parent, source))?;
    temporary
        .write_all(text.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| io_error(path, source))?;
    let (_, temporary_path) = temporary
        .keep()
        .map_err(|error| io_error(path, error.error))?;
    replace(&temporary_path, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary_path);
    })
}

#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> Result<(), DrainMarkerError> {
    cdr_windows_native::atomic_replace(source, destination)
        .map_err(|error| io_error(destination, std::io::Error::other(error)))
}

#[cfg(not(windows))]
fn replace(source: &Path, destination: &Path) -> Result<(), DrainMarkerError> {
    fs::rename(source, destination).map_err(|source| io_error(destination, source))
}

fn remove_file(path: &Path) -> Result<(), DrainMarkerError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn malformed(path: &Path, reason: &str) -> DrainMarkerError {
    DrainMarkerError::Malformed {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> DrainMarkerError {
    DrainMarkerError::Io {
        path: path.to_owned(),
        source,
    }
}
