use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::platform::{
    RootGuard, file_identity, link_count_for_path, open_update, path_identity, verify_open_regular,
};
use super::{RemoteFileError, hex_digest};

pub fn write(
    guard: &RootGuard,
    relative: &Path,
    content: &[u8],
    expected_sha256: Option<&str>,
) -> Result<bool, RemoteFileError> {
    let (target, _parents) = guard.target_with_parents(relative, true)?;
    let created = validate_expected(&target, expected_sha256)?;
    let (temporary, mut retained_temporary) = write_temporary(&target, content)?;
    let temporary_identity = file_identity(&retained_temporary)?;
    let result = if created {
        publish_new(&temporary, &target)
    } else {
        publish_existing(
            &temporary,
            &target,
            expected_sha256.expect("existing target has expected hash"),
        )
    };
    if let Err(error) = result {
        drop(retained_temporary);
        remove_if_identity(&temporary, &temporary_identity);
        return Err(error);
    }
    retained_temporary.seek(SeekFrom::Start(0))?;
    let mut published = Vec::new();
    retained_temporary.read_to_end(&mut published)?;
    if published != content || path_identity(&target)? != temporary_identity {
        return Err(RemoteFileError::conflict(
            &target,
            "file changed during the atomic update",
        ));
    }
    if link_count_for_path(&target)? != 2 {
        return Err(RemoteFileError::conflict(
            &target,
            "published file has an unexpected hard-link count",
        ));
    }
    drop(retained_temporary);
    fs::remove_file(&temporary)?;
    Ok(created)
}

pub fn delete(
    guard: &RootGuard,
    relative: &Path,
    expected_sha256: &str,
) -> Result<(), RemoteFileError> {
    let (target, _parents) = guard.target_with_parents(relative, false)?;
    let mut retained = open_update(&target).map_err(|_| changed(&target))?;
    let retained_identity = file_identity(&retained)?;
    if hash_file(&mut retained)? != expected_sha256 || path_identity(&target)? != retained_identity
    {
        return Err(changed(&target));
    }
    let backup = sibling(&target, "delete");
    fs::rename(&target, &backup)?;
    if path_identity(&backup)? != retained_identity {
        restore_or_preserve(&backup, &target);
        return Err(changed(&target));
    }
    retained.seek(SeekFrom::Start(0))?;
    if hash_file(&mut retained)? != expected_sha256 {
        restore_or_preserve(&backup, &target);
        return Err(changed(&target));
    }
    if let Err(error) = fs::remove_file(&backup) {
        restore_or_preserve(&backup, &target);
        return Err(error.into());
    }
    Ok(())
}

fn validate_expected(
    target: &Path,
    expected_sha256: Option<&str>,
) -> Result<bool, RemoteFileError> {
    let mut current = match open_update(target) {
        Ok(file) => file,
        Err(RemoteFileError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            if expected_sha256.is_some() {
                return Err(changed(target));
            }
            return Ok(true);
        }
        Err(error) => return Err(error),
    };
    if expected_sha256.is_none() {
        return Err(RemoteFileError::conflict(
            target,
            "existing files require expected_sha256",
        ));
    }
    if hash_file(&mut current)? != expected_sha256.expect("checked") {
        return Err(changed(target));
    }
    Ok(false)
}

fn write_temporary(target: &Path, content: &[u8]) -> Result<(PathBuf, File), RemoteFileError> {
    for _ in 0..32 {
        let temporary = sibling(target, "tmp");
        let mut file = match create_temporary(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        verify_open_regular(&temporary, &file)?;
        file.write_all(content)?;
        file.flush()?;
        file.sync_all()?;
        file.seek(SeekFrom::Start(0))?;
        if hash_file(&mut file)? != hex_digest(content) {
            return Err(std::io::Error::other("temporary file verification failed").into());
        }
        return Ok((temporary, file));
    }
    Err(std::io::Error::other("could not reserve a unique temporary file").into())
}

fn publish_new(temporary: &Path, target: &Path) -> Result<(), RemoteFileError> {
    fs::hard_link(temporary, target).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            RemoteFileError::conflict(target, "file appeared before it was created")
        } else {
            error.into()
        }
    })
}

fn publish_existing(
    temporary: &Path,
    target: &Path,
    expected_sha256: &str,
) -> Result<(), RemoteFileError> {
    let mut retained = open_update(target).map_err(|_| changed(target))?;
    let retained_identity = file_identity(&retained)?;
    if hash_file(&mut retained)? != expected_sha256 {
        return Err(changed(target));
    }
    if path_identity(target)? != retained_identity {
        return Err(changed(target));
    }
    let backup = sibling(target, "bak");
    fs::rename(target, &backup)?;
    if file_identity(&retained)? != retained_identity
        || path_identity(&backup)? != retained_identity
    {
        restore_or_preserve(&backup, target);
        return Err(changed(target));
    }
    retained.seek(SeekFrom::Start(0))?;
    if hash_file(&mut retained)? != expected_sha256 {
        restore_or_preserve(&backup, target);
        return Err(changed(target));
    }
    if let Err(error) = fs::hard_link(temporary, target) {
        if target.exists() {
            let _ = fs::remove_file(&backup);
            return Err(RemoteFileError::conflict(
                target,
                "file changed during the atomic update",
            ));
        }
        restore_or_preserve(&backup, target);
        return Err(error.into());
    }
    fs::remove_file(&backup)?;
    Ok(())
}

fn restore_or_preserve(backup: &Path, target: &Path) {
    if !target.exists() {
        let _ = fs::rename(backup, target);
    }
}

fn remove_if_identity(path: &Path, expected: &same_file::Handle) {
    if path_identity(path).is_ok_and(|identity| &identity == expected) {
        let _ = fs::remove_file(path);
    }
}

fn hash_file(file: &mut File) -> Result<String, RemoteFileError> {
    file.seek(SeekFrom::Start(0))?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(hex_digest(&content))
}

fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{name}.{}.{}", Uuid::new_v4().simple(), suffix))
}

fn changed(target: &Path) -> RemoteFileError {
    RemoteFileError::conflict(target, "file changed since it was read")
}

#[cfg(windows)]
fn create_temporary(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(0x1 | 0x4)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(not(windows))]
fn create_temporary(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}
