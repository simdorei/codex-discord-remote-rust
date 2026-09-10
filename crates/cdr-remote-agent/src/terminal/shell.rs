use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use cdr_remote_protocol::output::TerminalCwdScope;
use cdr_remote_protocol::request::TerminalShell;

use super::TerminalError;

const SECRET_ENV_MARKERS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "COOKIE",
    "CREDENTIAL",
    "OTP",
];

pub fn arguments(
    requested: TerminalShell,
    command: &str,
) -> Result<(TerminalShell, Vec<String>), TerminalError> {
    let selected = match requested {
        TerminalShell::Auto if cfg!(windows) => TerminalShell::Powershell,
        TerminalShell::Auto => TerminalShell::Sh,
        value => value,
    };
    let (executable, flags): (PathBuf, &[&str]) = match selected {
        TerminalShell::Powershell => (
            find_first(&["pwsh", "powershell"]).ok_or(TerminalError::ShellUnavailable(selected))?,
            &["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"],
        ),
        TerminalShell::Cmd => (
            env::var_os("ComSpec")
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .or_else(|| find_first(&["cmd"]))
                .ok_or(TerminalError::ShellUnavailable(selected))?,
            &["/D", "/S", "/C"],
        ),
        TerminalShell::Sh => (
            find_first(&["sh"]).ok_or(TerminalError::ShellUnavailable(selected))?,
            &["-c"],
        ),
        TerminalShell::Bash => (
            find_first(&["bash"]).ok_or(TerminalError::ShellUnavailable(selected))?,
            &["-c"],
        ),
        TerminalShell::Auto => unreachable!(),
    };
    Ok((
        selected,
        std::iter::once(executable.display().to_string())
            .chain(flags.iter().map(|value| (*value).to_owned()))
            .chain(std::iter::once(command.to_owned()))
            .collect(),
    ))
}

pub fn inherited_environment() -> HashMap<String, String> {
    env::vars()
        .filter(|(name, _)| {
            let upper = name.to_ascii_uppercase();
            !name.contains('=')
                && !SECRET_ENV_MARKERS
                    .iter()
                    .any(|marker| upper.contains(marker))
        })
        .collect()
}

pub fn resolve_cwd(current: &Path, requested: Option<&str>) -> Result<PathBuf, TerminalError> {
    let candidate = requested.map_or_else(
        || current.to_path_buf(),
        |value| {
            let path = Path::new(value);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                current.join(path)
            }
        },
    );
    let resolved = candidate
        .canonicalize()
        .map_err(|_| TerminalError::MissingExecutableOrDirectory)?;
    if resolved.is_dir() {
        Ok(resolved)
    } else {
        Err(TerminalError::MissingExecutableOrDirectory)
    }
}

pub fn cwd_scope(root: &Path, cwd: &Path) -> TerminalCwdScope {
    match cwd.strip_prefix(root) {
        Ok(relative) if relative.as_os_str().is_empty() => TerminalCwdScope::ProjectRoot,
        Ok(_) => TerminalCwdScope::ProjectRelative,
        Err(_) => TerminalCwdScope::ExternalAbsolute,
    }
}

pub fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = value.strip_prefix(r"\\?\") {
            return rest.to_owned();
        }
    }
    value.into_owned()
}

#[cfg(windows)]
pub fn window_arguments(
    shell: cdr_remote_protocol::request::TerminalWindowShell,
    title: &str,
) -> Result<(PathBuf, Vec<String>), TerminalError> {
    use cdr_remote_protocol::request::TerminalWindowShell;

    match shell {
        TerminalWindowShell::Powershell => Ok((
            find_first(&["pwsh", "powershell"])
                .ok_or(TerminalError::ShellUnavailable(TerminalShell::Powershell))?,
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NoExit".into(),
                "-Command".into(),
                "$Host.UI.RawUI.WindowTitle=$env:SIMDOREI_MCP_TERMINAL_WINDOW_TITLE".into(),
            ],
        )),
        TerminalWindowShell::Cmd => Ok((
            env::var_os("ComSpec")
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .or_else(|| find_first(&["cmd"]))
                .ok_or(TerminalError::ShellUnavailable(TerminalShell::Cmd))?,
            vec!["/D".into(), "/K".into(), format!("title {title}")],
        )),
    }
}

fn find_first(names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|name| find_in_path(name))
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_path_buf());
    }
    let extensions: Vec<String> = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|directory| {
            extensions.iter().find_map(|extension| {
                let candidate = directory.join(format!("{name}{extension}"));
                candidate.is_file().then_some(candidate)
            })
        })
    })
}
