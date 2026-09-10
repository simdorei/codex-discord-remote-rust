use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cdr_remote_protocol::output::CheckpointEntry;
use similar::TextDiff;

use crate::files::internal::CheckpointStore;
use crate::files::redaction::redact;
use crate::files::{ProjectFileAccess, RemoteFileError, hex_digest};

use super::transaction::{MutationTracker, begin, rollback};
use super::{CheckpointError, MAX_CHECKPOINT_BYTES, MAX_RECORD_BYTES, StoredCheckpoint};

const MAX_SCAN: usize = 10_000;
const MAX_RESULTS: usize = 50;
const MAX_RETAINED: usize = 500;
const MAX_STORAGE_BYTES: u64 = 536_870_912;

pub fn persist(root: &std::path::Path, record: &StoredCheckpoint) -> Result<(), CheckpointError> {
    let serialized = serde_json::to_vec(record)?;
    if serialized.len() > MAX_RECORD_BYTES {
        return Err(RemoteFileError::Size {
            path: record.checkpoint_id.clone(),
            reason: "checkpoint record is too large".into(),
        }
        .into());
    }
    let _lock = checkpoint_lock().lock().expect("checkpoint lock");
    let store = CheckpointStore::open(root)?;
    prune(&store, serialized.len() as u64)?;
    store.write_new(&format!("{}.json", record.checkpoint_id), &serialized)?;
    Ok(())
}

pub fn list(root: &std::path::Path) -> Result<Vec<CheckpointEntry>, CheckpointError> {
    let store = CheckpointStore::open(root)?;
    let mut candidates = candidates(&store)?;
    candidates.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
    let mut output = candidates
        .into_iter()
        .take(MAX_RESULTS)
        .map(|entry| load_from(&store, checkpoint_id(&entry.name).expect("filtered")))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|record| record.entry())
        .collect::<Vec<_>>();
    output.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(output)
}

pub fn show(
    root: &std::path::Path,
    checkpoint_id: &str,
) -> Result<(CheckpointEntry, String), CheckpointError> {
    let record = load(root, checkpoint_id)?;
    let mut patch = String::new();
    for snapshot in &record.snapshots {
        let before = STANDARD.decode(&snapshot.before_base64)?;
        let after = STANDARD.decode(&snapshot.after_base64)?;
        match (std::str::from_utf8(&before), std::str::from_utf8(&after)) {
            (Ok(before), Ok(after)) => patch.push_str(
                &TextDiff::from_lines(before, after)
                    .unified_diff()
                    .header(
                        &format!("a/{}", snapshot.path),
                        &format!("b/{}", snapshot.path),
                    )
                    .to_string(),
            ),
            _ => {
                let _ = writeln!(patch, "Binary file changed: {}", snapshot.path);
            }
        }
    }
    patch.truncate(patch.floor_char_boundary(MAX_CHECKPOINT_BYTES.min(patch.len())));
    Ok((record.entry(), redact(&patch)))
}

pub fn restore(
    access: &ProjectFileAccess,
    checkpoint_id: &str,
) -> Result<Vec<String>, CheckpointError> {
    let record = load(access.root(), checkpoint_id)?;
    let mut decoded = Vec::new();
    for snapshot in &record.snapshots {
        let before = STANDARD.decode(&snapshot.before_base64)?;
        let after = STANDARD.decode(&snapshot.after_base64)?;
        let exists = access.file_exists(&snapshot.path)?;
        if exists != snapshot.after_exists
            || (exists && access.read_bytes(&snapshot.path, MAX_CHECKPOINT_BYTES)? != after)
        {
            return Err(RemoteFileError::Conflict {
                path: snapshot.path.clone(),
                reason: "file changed after checkpoint was created".into(),
            }
            .into());
        }
        decoded.push((snapshot, before, after));
    }
    let paths = record
        .snapshots
        .iter()
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();
    let rollback_draft = begin(
        access,
        &format!("rollback failed restore {checkpoint_id}"),
        &paths,
    )?;
    let mut tracker = MutationTracker::default();
    let result = apply_restore(access, &decoded, &mut tracker);
    if let Err(error) = result {
        rollback(access, &rollback_draft, &tracker)?;
        return Err(error);
    }
    Ok(paths)
}

fn apply_restore(
    access: &ProjectFileAccess,
    decoded: &[(&super::StoredSnapshot, Vec<u8>, Vec<u8>)],
    tracker: &mut MutationTracker,
) -> Result<(), CheckpointError> {
    for (snapshot, before, after) in decoded {
        if snapshot.before_exists {
            access.write_bytes(
                &snapshot.path,
                before,
                MAX_CHECKPOINT_BYTES,
                snapshot.after_exists.then(|| hex_digest(after)).as_deref(),
            )?;
            tracker.record_write(&snapshot.path, hex_digest(before));
        } else if snapshot.after_exists {
            access.delete_file(&snapshot.path, &hex_digest(after))?;
            tracker.record_delete(&snapshot.path);
        }
    }
    Ok(())
}

fn load(root: &std::path::Path, id: &str) -> Result<StoredCheckpoint, CheckpointError> {
    if !valid_id(id) {
        return Err(CheckpointError::Invalid(id.to_owned()));
    }
    load_from(&CheckpointStore::open(root)?, id)
}

fn load_from(store: &CheckpointStore, id: &str) -> Result<StoredCheckpoint, CheckpointError> {
    let raw = store.read(&format!("{id}.json"), MAX_RECORD_BYTES)?;
    let record: StoredCheckpoint = serde_json::from_slice(&raw)?;
    if record.checkpoint_id != id || !valid_id(&record.checkpoint_id) {
        return Err(CheckpointError::Invalid(id.to_owned()));
    }
    Ok(record)
}

fn candidates(
    store: &CheckpointStore,
) -> Result<Vec<crate::files::internal::InternalEntry>, CheckpointError> {
    let entries = store.entries()?;
    if entries.len() > MAX_SCAN {
        return Err(RemoteFileError::Limit {
            pattern: ".codex-remote-mcp/checkpoints".into(),
            reason: "checkpoint listing scanned too many candidates".into(),
        }
        .into());
    }
    Ok(entries
        .into_iter()
        .filter(|entry| checkpoint_id(&entry.name).is_some())
        .collect())
}

fn prune(store: &CheckpointStore, incoming: u64) -> Result<(), CheckpointError> {
    let mut entries = candidates(store)?;
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
    let mut count = 0;
    let mut bytes = 0;
    for entry in entries {
        if count < MAX_RETAINED - 1 && bytes + entry.size <= MAX_STORAGE_BYTES - incoming {
            count += 1;
            bytes += entry.size;
        } else {
            store.remove(&entry.name)?;
        }
    }
    Ok(())
}

fn checkpoint_id(name: &str) -> Option<&str> {
    name.strip_suffix(".json").filter(|id| valid_id(id))
}

fn valid_id(id: &str) -> bool {
    id.len() == 19
        && id.starts_with("cp_")
        && id[3..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn checkpoint_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
