use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use cdr_app_server::AppServerError;
use sha2::{Digest, Sha256};

use super::held;

const FILES: [(&str, &[u8]); 2] = [
    ("authority.mjs", include_bytes!("authority.mjs")),
    ("launcher.mjs", include_bytes!("launcher.mjs")),
];

pub(super) fn digest() -> String {
    let mut hash = Sha256::new();
    for (name, contents) in FILES {
        hash.update(name);
        hash.update([0]);
        hash.update(contents);
        hash.update([0]);
    }
    hex::encode(hash.finalize())
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn directory(path: &Path) -> Result<(), AppServerError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if linked(&metadata) || !metadata.is_dir() {
        return Err(held("browser host cache directory must not be linked"));
    }
    Ok(())
}

fn verify(path: &Path, expected: &[u8]) -> Result<(), AppServerError> {
    let metadata = fs::symlink_metadata(path)?;
    if linked(&metadata) || !metadata.is_file() {
        return Err(held("browser host cache artifact must be a regular file"));
    }
    let mut contents = Vec::new();
    File::open(path)?
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut contents)?;
    if contents != expected {
        return Err(held(
            "browser host cache artifact differs from this runtime",
        ));
    }
    Ok(())
}

pub(super) fn prepare(root: &Path) -> Result<PathBuf, AppServerError> {
    if !root.is_absolute() {
        return Err(held("browser host cache must be absolute"));
    }
    directory(root)?;
    let version = root.join(digest());
    directory(&version)?;
    for (name, contents) in FILES {
        let path = version.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut pending = tempfile::NamedTempFile::new_in(&version)?;
                pending.write_all(contents)?;
                pending.as_file().sync_all()?;
                match pending.persist_noclobber(&path) {
                    Ok(_) => {}
                    Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        }
        // Another starter can publish the same bytes, but changed existing
        // artifacts are never overwritten or silently accepted.
        verify(&path, contents)?;
    }
    Ok(version.join("launcher.mjs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_publication_is_complete_and_reusable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache with spaces");
        let launchers = std::thread::scope(|scope| {
            let workers = (0..4)
                .map(|_| scope.spawn(|| prepare(&root).unwrap()))
                .collect::<Vec<_>>();
            workers
                .into_iter()
                .map(|w| w.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(launchers.iter().all(|path| path == &launchers[0]));
        assert_eq!(prepare(&root).unwrap(), launchers[0]);
        for (name, contents) in FILES {
            assert_eq!(fs::read(root.join(digest()).join(name)).unwrap(), contents);
        }
    }

    #[test]
    fn changed_existing_artifact_is_held_without_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let launcher = prepare(&root).unwrap();
        fs::write(&launcher, "changed").unwrap();
        assert!(prepare(&root).unwrap_err().to_string().contains("differs"));
        assert_eq!(fs::read_to_string(launcher).unwrap(), "changed");
    }

    #[test]
    fn a_non_directory_cache_is_rejected_without_modification() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        fs::write(&root, "existing user file").unwrap();
        assert!(prepare(&root).is_err());
        assert_eq!(fs::read_to_string(root).unwrap(), "existing user file");
    }
}
