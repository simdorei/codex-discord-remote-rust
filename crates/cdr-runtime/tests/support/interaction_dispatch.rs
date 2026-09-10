use std::path::{Path, PathBuf};

pub struct DispatchDatabase {
    _root: tempfile::TempDir,
    path: PathBuf,
}

impl DispatchDatabase {
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("create interaction custody directory");
        let path = root.path().join("mirror.sqlite");
        Self { _root: root, path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
