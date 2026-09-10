use std::collections::HashSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::files::{ProjectFileAccess, RemoteFileError, hex_digest};

use super::store::persist;
use super::{BeforeSnapshot, CheckpointDraft, CheckpointError, MAX_CHECKPOINT_BYTES};

#[derive(Clone)]
struct ProducedState {
    path: String,
    exists: bool,
    sha256: Option<String>,
}

#[derive(Default)]
pub struct MutationTracker {
    produced: Vec<ProducedState>,
}

impl MutationTracker {
    pub fn record_write(&mut self, path: &str, sha256: String) {
        self.produced.push(ProducedState {
            path: path.to_owned(),
            exists: true,
            sha256: Some(sha256),
        });
    }

    pub fn record_delete(&mut self, path: &str) {
        self.produced.push(ProducedState {
            path: path.to_owned(),
            exists: false,
            sha256: None,
        });
    }
}

pub fn mutate<T>(
    access: &ProjectFileAccess,
    reason: &str,
    paths: &[String],
    apply: impl FnOnce(&mut MutationTracker) -> Result<T, CheckpointError>,
) -> Result<(T, String), CheckpointError> {
    let draft = begin(access, reason, paths)?;
    let mut tracker = MutationTracker::default();
    let value = match apply(&mut tracker) {
        Ok(value) => value,
        Err(error) => {
            rollback(access, &draft, &tracker)?;
            return Err(error);
        }
    };
    match finish(access, &draft) {
        Ok(checkpoint_id) => Ok((value, checkpoint_id)),
        Err(error) => {
            rollback(access, &draft, &tracker)?;
            Err(error)
        }
    }
}

pub(super) fn begin(
    access: &ProjectFileAccess,
    reason: &str,
    paths: &[String],
) -> Result<CheckpointDraft, CheckpointError> {
    let mut unique = HashSet::new();
    let mut snapshots = Vec::new();
    let mut total = 0;
    for path in paths {
        if !unique.insert(path.clone()) {
            continue;
        }
        let existed = access.file_exists(path)?;
        let remaining = MAX_CHECKPOINT_BYTES.saturating_sub(total);
        let content = if existed {
            access.read_bytes(path, remaining)?
        } else {
            Vec::new()
        };
        total += content.len();
        snapshots.push(BeforeSnapshot {
            path: path.clone(),
            existed,
            content,
        });
    }
    let created_at = Utc::now().to_rfc3339();
    let seed = paths.join("\0");
    let digest = format!(
        "{:x}",
        Sha256::digest(format!("{created_at}\0{reason}\0{seed}").as_bytes())
    );
    Ok(CheckpointDraft {
        checkpoint_id: format!("cp_{}", &digest[..16]),
        created_at,
        reason: reason.to_owned(),
        snapshots,
    })
}

fn finish(access: &ProjectFileAccess, draft: &CheckpointDraft) -> Result<String, CheckpointError> {
    let mut total = 0;
    let mut snapshots = Vec::new();
    for snapshot in &draft.snapshots {
        let after_exists = access.file_exists(&snapshot.path)?;
        let remaining = MAX_CHECKPOINT_BYTES
            .saturating_mul(2)
            .saturating_sub(total + snapshot.content.len());
        let after = if after_exists {
            access.read_bytes(&snapshot.path, remaining)?
        } else {
            Vec::new()
        };
        total += snapshot.content.len() + after.len();
        if total > MAX_CHECKPOINT_BYTES * 2 {
            return Err(RemoteFileError::Size {
                path: snapshot.path.clone(),
                reason: "checkpoint before/after data is too large".into(),
            }
            .into());
        }
        snapshots.push(super::StoredSnapshot {
            path: snapshot.path.clone(),
            before_exists: snapshot.existed,
            before_base64: STANDARD.encode(&snapshot.content),
            after_exists,
            after_base64: STANDARD.encode(after),
        });
    }
    persist(
        access.root(),
        &super::StoredCheckpoint {
            checkpoint_id: draft.checkpoint_id.clone(),
            created_at: draft.created_at.clone(),
            reason: draft.reason.clone(),
            snapshots,
        },
    )?;
    Ok(draft.checkpoint_id.clone())
}

pub(super) fn rollback(
    access: &ProjectFileAccess,
    draft: &CheckpointDraft,
    tracker: &MutationTracker,
) -> Result<(), CheckpointError> {
    for state in &tracker.produced {
        validate_state(access, state)?;
    }
    for snapshot in draft.snapshots.iter().rev() {
        let Some(state) = tracker
            .produced
            .iter()
            .rev()
            .find(|item| item.path == snapshot.path)
        else {
            continue;
        };
        if snapshot.existed {
            access.write_bytes(
                &snapshot.path,
                &snapshot.content,
                MAX_CHECKPOINT_BYTES,
                state.sha256.as_deref(),
            )?;
        } else if state.exists {
            access.delete_file(&snapshot.path, state.sha256.as_deref().unwrap_or(""))?;
        }
    }
    Ok(())
}

fn validate_state(
    access: &ProjectFileAccess,
    state: &ProducedState,
) -> Result<(), CheckpointError> {
    let exists = access.file_exists(&state.path)?;
    if exists != state.exists {
        return Err(RemoteFileError::Conflict {
            path: state.path.clone(),
            reason: "file changed during rollback".into(),
        }
        .into());
    }
    if exists
        && hex_digest(&access.read_bytes(&state.path, MAX_CHECKPOINT_BYTES)?)
            != state.sha256.clone().unwrap_or_default()
    {
        return Err(RemoteFileError::Conflict {
            path: state.path.clone(),
            reason: "file changed during rollback".into(),
        }
        .into());
    }
    Ok(())
}
