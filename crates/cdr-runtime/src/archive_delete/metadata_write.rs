//! Windows excludes other writers/replacements throughout validation and rewrite.
//! In-place writes are recoverable from the already verified archive backup.
use super::{ArchiveDeleteError, io_error};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

pub(super) fn replace_checked(
    path: &Path,
    bytes: &[u8],
    original: &[u8],
) -> Result<(), ArchiveDeleteError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let mut file = options.open(path).map_err(|e| io_error(path, e))?;
    // Advisory on Unix; callers on that platform must quiesce noncooperating writers.
    file.try_lock()
        .map_err(|e| io_error(path, std::io::Error::other(e)))?;
    let mut current = Vec::new();
    file.read_to_end(&mut current)
        .map_err(|e| io_error(path, e))?;
    if current != original {
        return Err(ArchiveDeleteError::ConcurrentMutation);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_error(path, e))?;
    file.write_all(bytes).map_err(|e| io_error(path, e))?;
    file.set_len(bytes.len() as u64)
        .map_err(|e| io_error(path, e))?;
    file.sync_all().map_err(|e| io_error(path, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_contents_are_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        std::fs::write(&path, b"new concurrent state").unwrap();
        assert!(matches!(
            replace_checked(&path, b"rewrite", b"old state"),
            Err(ArchiveDeleteError::ConcurrentMutation)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"new concurrent state");
    }
    #[test]
    fn held_file_is_rejected_without_modification() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        std::fs::write(&path, b"original").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        held.try_lock().unwrap();
        assert!(replace_checked(&path, b"replacement", b"original").is_err());
        drop(held);
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }
}
