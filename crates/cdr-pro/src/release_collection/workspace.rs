use super::Runner;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};
pub(super) struct Snapshot {
    pub revision: String,
    pub state: String,
    pub digest: Option<Vec<u8>>,
}
pub(super) fn snapshot(root: &Path, runner: &mut impl Runner) -> Snapshot {
    let revision = runner.command(&["git", "rev-parse", "HEAD"], root);
    let state = runner.command(&["git", "status", "--porcelain"], root);
    let tracked = runner.command(
        &["git", "diff", "--binary", "--no-ext-diff", "HEAD", "--"],
        root,
    );
    let untracked = runner.command(
        &["git", "ls-files", "--others", "--exclude-standard", "-z"],
        root,
    );
    let mut digest = Sha256::new();
    digest.update(tracked.stdout.as_bytes());
    let content = hash_untracked(root, &untracked.stdout, &mut digest);
    Snapshot {
        revision: if revision.code == 0 {
            revision.stdout.trim().into()
        } else {
            String::new()
        },
        state: if state.code != 0 {
            "unavailable"
        } else if state.stdout.trim().is_empty() {
            "clean"
        } else {
            "dirty"
        }
        .into(),
        digest: (tracked.code == 0 && untracked.code == 0 && state.code == 0 && content.is_ok())
            .then(|| digest.finalize().to_vec()),
    }
}
fn hash_untracked(root: &Path, paths: &str, digest: &mut Sha256) -> std::io::Result<()> {
    let root = root.canonicalize()?;
    let mut paths: Vec<_> = paths.split('\0').filter(|v| !v.is_empty()).collect();
    paths.sort_unstable();
    for relative in paths {
        let path = root.join(relative);
        if path.symlink_metadata()?.file_type().is_symlink() {
            return Err(std::io::Error::other("untracked symlink"));
        }
        let resolved = path.canonicalize()?;
        if !resolved.starts_with(&root) || !resolved.is_file() {
            return Err(std::io::Error::other("untracked file escaped root"));
        }
        let key = relative.replace('\\', "/");
        digest.update((key.len() as u64).to_be_bytes());
        digest.update(key.as_bytes());
        let mut source = File::open(resolved)?;
        let mut buffer = vec![0; 65_536];
        loop {
            let length = source.read(&mut buffer)?;
            if length == 0 {
                break;
            }
            digest.update(&buffer[..length]);
        }
    }
    Ok(())
}
