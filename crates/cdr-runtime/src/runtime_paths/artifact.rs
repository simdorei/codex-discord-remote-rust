use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use super::{RuntimePathError, selection::failed};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

pub(super) fn locked_file(path: &Path) -> Result<File, RuntimePathError> {
    // FILE_SHARE_READ only: no writer or rename/delete can race the retained lease.
    OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|error| failed(format!("artifact unavailable {}: {error}", path.display())))
}

fn fingerprint(
    path: &Path,
    file: &mut File,
    deadline: Instant,
) -> Result<ArtifactFile, RuntimePathError> {
    let resolved =
        cdr_windows_native::resolved_file_path(file).map_err(|e| failed(e.to_string()))?;
    if !cdr_windows_native::same_resolved_path(path, &resolved) {
        return Err(failed(format!(
            "artifact path indirection is not stable: {}",
            path.display()
        )));
    }
    let mut hash = Sha256::new();
    let mut block = vec![0_u8; 65_536];
    let mut bytes = 0_u64;
    loop {
        if Instant::now() >= deadline {
            return Err(failed("artifact_probe_timeout"));
        }
        let count = file
            .read(&mut block)
            .map_err(|error| failed(format!("artifact read: {error}")))?;
        if count == 0 {
            break;
        }
        hash.update(&block[..count]);
        bytes += count as u64;
    }
    Ok(ArtifactFile {
        path: path.to_owned(),
        bytes,
        sha256: hex::encode(hash.finalize()),
    })
}

pub(super) fn observe(
    path: &Path,
    require_host: bool,
    deadline: Instant,
) -> Result<Vec<ArtifactFile>, RuntimePathError> {
    let mut files = vec![];
    for name in [
        None,
        Some("codex-code-mode-host.exe"),
        Some("codex-command-runner.exe"),
        Some("codex-windows-sandbox-setup.exe"),
    ] {
        let candidate = name.map_or_else(|| path.to_owned(), |name| path.with_file_name(name));
        match std::fs::metadata(&candidate) {
            Ok(meta) if meta.is_file() => {}
            Err(error)
                if name.is_some()
                    && !(name == Some("codex-code-mode-host.exe") && require_host)
                    && error.kind() == std::io::ErrorKind::NotFound =>
            {
                continue;
            }
            other => {
                return Err(failed(format!(
                    "artifact observation {}: {other:?}",
                    candidate.display()
                )));
            }
        }
        files.push(fingerprint(
            &candidate,
            &mut locked_file(&candidate)?,
            deadline,
        )?);
    }
    Ok(files)
}

pub(super) fn lease(
    files: &[ArtifactFile],
    deadline: Instant,
) -> Result<Vec<Arc<File>>, RuntimePathError> {
    files
        .iter()
        .map(|expected| {
            let mut file = locked_file(&expected.path)?;
            if fingerprint(&expected.path, &mut file, deadline)? != *expected {
                return Err(failed(format!(
                    "artifact_changed: {}",
                    expected.path.display()
                )));
            }
            Ok(Arc::new(file))
        })
        .collect()
}
