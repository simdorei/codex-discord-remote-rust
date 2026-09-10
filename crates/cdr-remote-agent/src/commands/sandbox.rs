use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use regex::Regex;

use super::{CommandError, rejected};

const ENVIRONMENT_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SystemDrive",
    "SystemRoot",
    "ProgramData",
    "COMSPEC",
    "ComSpec",
    "PATHEXT",
    "LOCALAPPDATA",
    "APPDATA",
    "USERPROFILE",
    "SSH_AUTH_SOCK",
    "CODEX_HOME",
];

pub fn sandbox_arguments(
    root: &Path,
    arguments: &[String],
    codex_exe: Option<&str>,
) -> Result<Vec<String>, CommandError> {
    if arguments.is_empty()
        || arguments
            .iter()
            .any(|argument| !safe_argument().is_match(argument))
    {
        return Err(rejected(
            "sandbox",
            "command arguments are not a fixed safe token list",
        ));
    }
    let prefix = match codex_exe {
        Some(value) if !value.is_empty() => codex_prefix(Path::new(value)),
        _ => find_codex_prefix(),
    }
    .ok_or_else(|| {
        rejected(
            "sandbox",
            "Codex CLI is required to run project commands safely",
        )
    })?;
    Ok([
        prefix,
        vec![
            "sandbox".into(),
            "-C".into(),
            root.display().to_string(),
            "-P".into(),
            ":workspace".into(),
            "--sandbox-state-disable-network".into(),
            "--".into(),
        ],
        arguments.to_vec(),
    ]
    .concat())
}

#[must_use]
pub fn safe_environment() -> HashMap<String, String> {
    ENVIRONMENT_ALLOWLIST
        .iter()
        .filter_map(|key| env::var(key).ok().map(|value| ((*key).to_owned(), value)))
        .collect()
}

fn find_codex_prefix() -> Option<Vec<String>> {
    if let Some(value) = env::var_os("CODEX_EXE") {
        return codex_prefix(Path::new(&value));
    }
    find_executable("codex").and_then(|path| codex_prefix(&path))
}

fn codex_prefix(path: &Path) -> Option<Vec<String>> {
    if !cfg!(windows)
        || path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Some(vec![path.display().to_string()]);
    }
    let script = path
        .parent()?
        .join("node_modules/@openai/codex/bin/codex.js");
    let node = find_executable("node")?;
    script
        .is_file()
        .then(|| vec![node.display().to_string(), script.display().to_string()])
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let executable = if cfg!(windows) { "where.exe" } else { "which" };
    let output = Command::new(executable).arg(name).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}

fn safe_argument() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^[A-Za-z0-9_./:@=+-]+$").expect("valid safe token"))
}
