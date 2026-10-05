//! Resolve the exact --env/cwd path with the same loader as normal startup.
use super::super::Args;
use crate::startup::{StartupArgs, load_environment};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(super) fn check(args: &Args, expected: &Path) -> Result<Option<PathBuf>, String> {
    let Some(raw) = args.value("--env") else {
        return Ok(None);
    };
    let environment_path = Path::new(raw)
        .canonicalize()
        .map_err(|error| format!("explicit environment is unavailable: {error}"))?;
    if !environment_path.is_file() {
        return Err("explicit environment is not a file".into());
    }
    let startup = StartupArgs::parse([
        OsString::from("--env"),
        environment_path.as_os_str().to_owned(),
    ])
    .map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let environment = load_environment(&startup, &executable).map_err(|error| error.to_string())?;
    let working_directory = std::env::current_dir().map_err(|error| error.to_string())?;
    let home = ["USERPROFILE", "HOME"]
        .into_iter()
        .filter_map(|name| environment.get(name))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(Path::new);
    let (_, configured) =
        crate::runtime_paths::store_paths::resolve(&environment, &working_directory, home)
            .map_err(|error| error.to_string())?;
    let configured = working_directory
        .join(configured)
        .canonicalize()
        .map_err(|error| format!("configured database is unavailable: {error}"))?;
    let expected = expected.canonicalize().map_err(|error| error.to_string())?;
    if configured != expected {
        return Err("configured database differs from the pinned compatibility database".into());
    }
    Ok(Some(environment_path))
}
