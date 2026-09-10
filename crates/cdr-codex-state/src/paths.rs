use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn resolve_state_db_path(codex_home: &Path) -> PathBuf {
    let mut candidates: Vec<(SystemTime, String, PathBuf)> = fs::read_dir(codex_home)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_owned();
            if !name.starts_with("state_") || !name.ends_with(".sqlite") {
                return None;
            }
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, name, path))
        })
        .collect();
    candidates.sort_by(|left, right| right.cmp(left));
    candidates
        .into_iter()
        .next()
        .map_or_else(|| codex_home.join("state_5.sqlite"), |(_, _, path)| path)
}
