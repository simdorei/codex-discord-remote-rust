//! Immutable ticket pins checked before database/RPC access, shared by both operators.
use cdr_runtime::runtime_paths::RuntimePaths;
use std::path::Path;

pub fn verify(
    paths: &RuntimePaths,
    legacy_root: &Path,
    home: &Path,
    state: &Path,
    mirror: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if paths.root.as_os_str() != legacy_root.as_os_str() {
        return Err("maintenance legacy mutex root spelling mismatch".into());
    }
    if paths.codex_home.canonicalize()? != home.canonicalize()?
        || paths.state_db.canonicalize()? != state.canonicalize()?
    {
        return Err("maintenance approved Codex home/state DB mismatch".into());
    }
    if paths.mirror_db.canonicalize()? != mirror.canonicalize()? {
        return Err("maintenance DB mismatch".into());
    }
    Ok(())
}
