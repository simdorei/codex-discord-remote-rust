use std::fs::{self, File, Metadata, OpenOptions, ReadDir};
use std::path::{Path, PathBuf};

use same_file::Handle;

use super::RemoteFileError;

pub struct RootGuard {
    root: PathBuf,
    canonical: PathBuf,
    identity: Handle,
    _retained: File,
}

pub struct LockedFile {
    pub file: File,
    #[allow(dead_code)]
    parents: Vec<File>,
}

impl RootGuard {
    pub fn open(root: &Path) -> Result<Self, RemoteFileError> {
        let canonical = fs::canonicalize(root)?;
        let metadata = fs::symlink_metadata(&canonical)?;
        verify_directory(&canonical, &metadata)?;
        let retained = open_directory(&canonical)?;
        let retained_metadata = retained.metadata()?;
        verify_directory(&canonical, &retained_metadata)?;
        Ok(Self {
            root: canonical.clone(),
            canonical,
            identity: identity(&retained)?,
            _retained: retained,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn verify(&self) -> Result<(), RemoteFileError> {
        let metadata = fs::symlink_metadata(&self.root).map_err(|error| {
            RemoteFileError::unsafe_path(
                &self.root,
                format!("the bound project root moved or changed identity: {error}"),
            )
        })?;
        verify_directory(&self.root, &metadata)?;
        if Handle::from_path(&self.root)? != self.identity
            || fs::canonicalize(&self.root)? != self.canonical
        {
            return Err(RemoteFileError::unsafe_path(
                &self.root,
                "the bound project root moved or changed identity",
            ));
        }
        Ok(())
    }

    pub fn open_regular(&self, relative: &Path) -> Result<LockedFile, RemoteFileError> {
        let (target, parents) = self.target_with_parents(relative, false)?;
        let file = open_regular(&target, false)?;
        verify_open_regular(&target, &file)?;
        self.verify_confined(&target)?;
        Ok(LockedFile { file, parents })
    }

    pub fn read_directory(&self, relative: &Path) -> Result<(ReadDir, Vec<File>), RemoteFileError> {
        let synthetic = relative.join("entry");
        let (target, parents) = self.target_with_parents(&synthetic, false)?;
        let directory = target.parent().expect("synthetic target has parent");
        let current = open_directory(directory)?;
        verify_directory(directory, &current.metadata()?)?;
        self.verify_confined(directory)?;
        let mut retained = parents;
        retained.push(current);
        Ok((fs::read_dir(directory)?, retained))
    }

    pub fn target_with_parents(
        &self,
        relative: &Path,
        create: bool,
    ) -> Result<(PathBuf, Vec<File>), RemoteFileError> {
        self.verify()?;
        let mut current = self.root.clone();
        let mut parents = Vec::new();
        if let Some(parent) = relative.parent() {
            for part in parent.components() {
                current.push(part.as_os_str());
                if create {
                    match fs::create_dir(&current) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                let retained = open_directory(&current)?;
                verify_directory(&current, &retained.metadata()?)?;
                self.verify_confined(&current)?;
                parents.push(retained);
            }
        }
        Ok((self.root.join(relative), parents))
    }

    fn verify_confined(&self, path: &Path) -> Result<(), RemoteFileError> {
        let canonical = fs::canonicalize(path)?;
        if !canonical.starts_with(&self.canonical) {
            return Err(RemoteFileError::unsafe_path(
                path,
                "path is outside the project root",
            ));
        }
        Ok(())
    }
}

pub fn open_update(path: &Path) -> Result<File, RemoteFileError> {
    let file = open_regular(path, true)?;
    verify_open_regular(path, &file)?;
    Ok(file)
}

pub fn verify_open_regular(path: &Path, file: &File) -> Result<(), RemoteFileError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err(RemoteFileError::unsafe_path(
            path,
            "path is not a regular file",
        ));
    }
    if link_count_for_path_impl(path)? != 1 {
        return Err(RemoteFileError::unsafe_path(
            path,
            "hard links are not allowed",
        ));
    }
    Ok(())
}

pub fn file_identity(file: &File) -> Result<Handle, RemoteFileError> {
    Ok(Handle::from_file(file.try_clone()?)?)
}

pub fn path_identity(path: &Path) -> Result<Handle, RemoteFileError> {
    Ok(Handle::from_path(path)?)
}

pub fn link_count_for_path(path: &Path) -> Result<u64, RemoteFileError> {
    link_count_for_path_impl(path)
}

fn verify_directory(path: &Path, metadata: &Metadata) -> Result<(), RemoteFileError> {
    if !metadata.is_dir() || is_reparse(metadata) {
        Err(RemoteFileError::unsafe_path(
            path,
            "project path is not a retained regular directory",
        ))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn open_directory(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .access_mode(0x80)
        .share_mode(0x1 | 0x2)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(path)
}

#[cfg(not(windows))]
fn open_directory(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
fn open_regular(path: &Path, update: bool) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    let share = if update { 0x1 | 0x4 } else { 0x1 | 0x2 };
    OpenOptions::new()
        .read(true)
        .share_mode(share)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(not(windows))]
fn open_regular(path: &Path, _update: bool) -> std::io::Result<File> {
    File::open(path)
}

fn identity(file: &File) -> Result<Handle, RemoteFileError> {
    Ok(Handle::from_file(file.try_clone()?)?)
}

#[cfg(windows)]
fn link_count_for_path_impl(path: &Path) -> Result<u64, RemoteFileError> {
    let output = std::process::Command::new("fsutil.exe")
        .args(["hardlink", "list"])
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "fsutil hardlink list failed with exit code {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    let count = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    Ok(count as u64)
}

#[cfg(unix)]
fn link_count_for_path_impl(path: &Path) -> Result<u64, RemoteFileError> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::metadata(path)?.nlink())
}

#[cfg(windows)]
pub fn is_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
pub fn is_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}
