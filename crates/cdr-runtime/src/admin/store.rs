use std::path::{Path, PathBuf};

fn configured_path(root: &Path) -> Result<PathBuf, String> {
    let environment = super::project_environment(root)?;
    let home = ["USERPROFILE", "HOME"]
        .into_iter()
        .filter_map(|name| environment.get(name))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(Path::new);
    let (_, path) = crate::runtime_paths::store_paths::resolve(&environment, root, home)
        .map_err(|error| error.to_string())?;
    Ok(std::env::current_dir()
        .map_err(|error| error.to_string())?
        .join(path))
}

pub(super) fn backup(root: &Path) -> Result<String, String> {
    let backup =
        cdr_store::backup::snapshot(&configured_path(root)?).map_err(|error| error.to_string())?;
    Ok(format!("backup_created path={}", backup.display()))
}

pub(super) fn active_count(root: &Path) -> Result<String, String> {
    let path = configured_path(root)?;
    let connection =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| {
                format!("Cannot inspect active queue at {}: {error}", path.display())
            })?;
    connection
        .busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|error| format!("Cannot configure queue inspection: {error}"))?;
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM codex_turn_queue WHERE state IN ('running', 'starting')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot inspect active queue: {error}"))?;
    Ok(count.to_string())
}
