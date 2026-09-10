use cdr_remote_protocol::file_change::{FileChange, FileChangeAction};
use cdr_remote_protocol::output::{CoreOutput, PatchAction, PatchEntry};
use similar::{ChangeTag, TextDiff};

use crate::checkpoints::{CheckpointError, mutate};
use crate::files::{MAX_FILE_BYTES, ProjectFileAccess, RemoteFileError, hex_digest};

struct PlannedMutation {
    entry: PatchEntry,
    content: Option<String>,
    expected_sha256: Option<String>,
}

pub fn apply(
    access: &ProjectFileAccess,
    changes: &[FileChange],
) -> Result<CoreOutput, CheckpointError> {
    let planned = changes
        .iter()
        .map(|change| plan(access, change))
        .collect::<Result<Vec<_>, _>>()?;
    let mut paths = Vec::new();
    for mutation in &planned {
        paths.push(mutation.entry.path.clone());
        if let Some(destination) = &mutation.entry.destination {
            paths.push(destination.clone());
        }
    }
    let ((), checkpoint_id) = mutate(access, "patch", &paths, |tracker| {
        for mutation in &planned {
            apply_one(access, mutation, tracker)?;
        }
        Ok(())
    })?;
    Ok(CoreOutput::FileApplyPatch {
        applied: planned.into_iter().map(|item| item.entry).collect(),
        checkpoint_id,
    })
}

fn plan(
    access: &ProjectFileAccess,
    change: &FileChange,
) -> Result<PlannedMutation, CheckpointError> {
    match change.action {
        FileChangeAction::Create => {
            if access.file_exists(&change.path)? {
                return Err(conflict(&change.path, "added file already exists").into());
            }
            let content = change.content.clone().unwrap_or_default();
            Ok(PlannedMutation {
                entry: PatchEntry {
                    path: change.path.clone(),
                    action: PatchAction::Add,
                    destination: None,
                    added_lines: content.lines().count() as u64,
                    removed_lines: 0,
                },
                content: Some(content),
                expected_sha256: None,
            })
        }
        FileChangeAction::Update | FileChangeAction::Delete | FileChangeAction::Move => {
            plan_existing(access, change)
        }
    }
}

fn plan_existing(
    access: &ProjectFileAccess,
    change: &FileChange,
) -> Result<PlannedMutation, CheckpointError> {
    let original = access.read_bytes(&change.path, MAX_FILE_BYTES)?;
    let expected = change.expected_sha256.clone().unwrap_or_default();
    if hex_digest(&original) != expected {
        return Err(conflict(&change.path, "file changed since it was read").into());
    }
    let original = String::from_utf8(original).map_err(|_| RemoteFileError::Encoding {
        path: change.path.clone(),
        reason: "file is not UTF-8 text".into(),
    })?;
    let content = match change.action {
        FileChangeAction::Delete => None,
        FileChangeAction::Move => Some(change.content.clone().unwrap_or_else(|| original.clone())),
        FileChangeAction::Update => change.content.clone(),
        FileChangeAction::Create => unreachable!(),
    };
    if let Some(destination) = &change.destination
        && access.file_exists(destination)?
    {
        return Err(conflict(destination, "move destination already exists").into());
    }
    let (added, removed) = content
        .as_deref()
        .map_or((0, original.lines().count() as u64), |updated| {
            delta(&original, updated)
        });
    Ok(PlannedMutation {
        entry: PatchEntry {
            path: change.path.clone(),
            action: match change.action {
                FileChangeAction::Update => PatchAction::Update,
                FileChangeAction::Delete => PatchAction::Delete,
                FileChangeAction::Move => PatchAction::Move,
                FileChangeAction::Create => unreachable!(),
            },
            destination: change.destination.clone(),
            added_lines: added,
            removed_lines: removed,
        },
        content,
        expected_sha256: Some(expected),
    })
}

fn apply_one(
    access: &ProjectFileAccess,
    mutation: &PlannedMutation,
    tracker: &mut crate::checkpoints::MutationTracker,
) -> Result<(), CheckpointError> {
    if let Some(expected) = &mutation.expected_sha256
        && hex_digest(&access.read_bytes(&mutation.entry.path, MAX_FILE_BYTES)?) != *expected
    {
        return Err(conflict(
            &mutation.entry.path,
            "file changed since the patch was planned",
        )
        .into());
    }
    match mutation.entry.action {
        PatchAction::Delete => {
            access.delete_file(
                &mutation.entry.path,
                mutation.expected_sha256.as_deref().unwrap_or(""),
            )?;
            tracker.record_delete(&mutation.entry.path);
        }
        PatchAction::Move => {
            let destination = mutation
                .entry
                .destination
                .as_deref()
                .expect("planned destination");
            let written =
                access.write_file(destination, mutation.content.as_deref().unwrap_or(""), None)?;
            tracker.record_write(destination, written.sha256);
            access.delete_file(
                &mutation.entry.path,
                mutation.expected_sha256.as_deref().unwrap_or(""),
            )?;
            tracker.record_delete(&mutation.entry.path);
        }
        PatchAction::Add | PatchAction::Update => {
            let written = access.write_file(
                &mutation.entry.path,
                mutation.content.as_deref().unwrap_or(""),
                mutation.expected_sha256.as_deref(),
            )?;
            tracker.record_write(&mutation.entry.path, written.sha256);
        }
    }
    Ok(())
}

fn delta(before: &str, after: &str) -> (u64, u64) {
    TextDiff::from_lines(before, after).iter_all_changes().fold(
        (0, 0),
        |(added, removed), change| match change.tag() {
            ChangeTag::Insert => (added + 1, removed),
            ChangeTag::Delete => (added, removed + 1),
            ChangeTag::Equal => (added, removed),
        },
    )
}

fn conflict(path: &str, reason: &str) -> RemoteFileError {
    RemoteFileError::Conflict {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
}
