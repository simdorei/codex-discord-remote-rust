//! Pin transcript identity from backup verification through deletion on Windows.
use super::{ArchiveDeleteError, io_error, verification};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub(super) struct Transcript {
    file: File,
    path: PathBuf,
}

impl Transcript {
    pub(super) fn verified(path: &Path, backup: &Path) -> Result<Self, ArchiveDeleteError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // GENERIC_READ | DELETE, deny concurrent readers, writers and renames.
            options.access_mode(0x8000_0000 | 0x0001_0000).share_mode(0);
        }
        let mut file = options.open(path).map_err(|e| io_error(path, e))?;
        file.try_lock()
            .map_err(|e| io_error(path, std::io::Error::other(e)))?;
        if verification::reader_digest(&mut file).map_err(|e| io_error(path, e))?
            != verification::file_digest(backup)?
        {
            return Err(ArchiveDeleteError::ConcurrentMutation);
        }
        Ok(Self {
            file,
            path: path.into(),
        })
    }

    pub(super) fn delete(self) -> Result<(), ArchiveDeleteError> {
        #[cfg(windows)]
        cdr_windows_native::delete_open_file(self.file).map_err(|e| io_error(&self.path, e))?;
        #[cfg(not(windows))]
        {
            // Advisory-lock platforms require quiesced noncooperating writers.
            std::fs::remove_file(&self.path).map_err(|e| io_error(&self.path, e))?;
            drop(self.file);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mismatched_backup_never_marks_the_transcript_for_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("original.jsonl");
        let backup = temp.path().join("backup.jsonl");
        std::fs::write(&path, b"current").unwrap();
        std::fs::write(&backup, b"old").unwrap();
        assert!(matches!(
            Transcript::verified(&path, &backup),
            Err(ArchiveDeleteError::ConcurrentMutation)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"current");
    }

    #[cfg(windows)]
    #[test]
    fn pinned_transcript_cannot_be_replaced_and_drop_without_commit_preserves_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("original.jsonl");
        let backup = temp.path().join("backup.jsonl");
        std::fs::write(&path, b"current").unwrap();
        std::fs::copy(&path, &backup).unwrap();
        let pinned = Transcript::verified(&path, &backup).unwrap();
        assert!(std::fs::write(&path, b"replacement").is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(pinned);
        assert_eq!(std::fs::read(&path).unwrap(), b"current");
    }
}
