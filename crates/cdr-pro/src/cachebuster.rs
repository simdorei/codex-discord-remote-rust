//! A plugin's cached native helper must change identity when its compiled inputs change.
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, process::Command};

pub const MANIFEST: &str = "plugins/codex-discord-remote/.codex-plugin/plugin.json";
const WATCHED: &[&str] = &[
    "plugins/codex-discord-remote",
    "crates/cdr-pro/src",
    "crates/cdr-pro/Cargo.toml",
    "crates/cdr-app-server/src",
    "crates/cdr-app-server/Cargo.toml",
    "crates/cdr-windows-native/src",
    "crates/cdr-windows-native/Cargo.toml",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".cargo",
];

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("cachebuster Git invocation failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cachebuster Git {} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| "cachebuster Git output was not UTF-8".into())
}
fn manifest(raw: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid plugin manifest: {e}"))?;
    if !value.is_object() || !value["version"].is_string() {
        return Err("invalid plugin manifest: object and string version required".into());
    }
    Ok(value)
}
fn version(raw: &str) -> Result<(u64, u64, u64, u64), String> {
    let regex = regex::Regex::new(
        r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\+codex\.([0-9]{14})$",
    )
    .map_err(|e| e.to_string())?;
    let captures = regex
        .captures(raw)
        .ok_or_else(|| format!("invalid Codex plugin version: {raw}"))?;
    let timestamp = &captures[4];
    if timestamp.starts_with("0000")
        || timestamp[12..14]
            .parse::<u8>()
            .map_or(true, |second| second >= 60)
        || chrono::NaiveDateTime::parse_from_str(timestamp, "%Y%m%d%H%M%S").is_err()
    {
        return Err(format!("invalid plugin cachebuster: {timestamp}"));
    }
    let number = |i: usize| {
        captures[i]
            .parse::<u64>()
            .map_err(|_| format!("invalid Codex plugin version: {raw}"))
    };
    Ok((number(1)?, number(2)?, number(3)?, number(4)?))
}

pub fn verify(repo: &Path, base_ref: &str, working_tree: bool) -> Result<(), String> {
    let revision = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base_ref}^{{commit}}"),
        ],
    )?;
    let base = git(repo, &["merge-base", "HEAD", revision.trim()])?;
    let base = base.trim();
    let mut before = manifest(&git(repo, &["show", &format!("{base}:{MANIFEST}")])?)?;
    let mut after = manifest(&if working_tree {
        std::fs::read_to_string(repo.join(MANIFEST)).map_err(|e| e.to_string())?
    } else {
        git(repo, &["show", &format!("HEAD:{MANIFEST}")])?
    })?;
    let old = version(before["version"].as_str().ok_or("missing old version")?)?;
    let new = version(after["version"].as_str().ok_or("missing new version")?)?;
    before
        .as_object_mut()
        .ok_or("invalid base manifest")?
        .remove("version");
    after
        .as_object_mut()
        .ok_or("invalid head manifest")?
        .remove("version");
    let mut args = vec!["diff", "--name-only", "-z", base];
    if !working_tree {
        args.push("HEAD");
    }
    args.push("--");
    args.extend_from_slice(WATCHED);
    let mut paths = git(repo, &args)?;
    if working_tree {
        let mut args = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
        args.extend_from_slice(WATCHED);
        paths.push_str(&git(repo, &args)?);
    }
    let payload_changed = before != after
        || paths
            .split('\0')
            .any(|path| !path.is_empty() && path != MANIFEST);
    if payload_changed && new <= old {
        return Err(format!(
            "packaged plugin content changed without an increasing plugin version: update {MANIFEST}"
        ));
    }
    if !payload_changed && new < old {
        return Err(format!(
            "plugin version changed without increasing its cache key: update {MANIFEST}"
        ));
    }
    Ok(())
}

pub fn run(arguments: impl IntoIterator<Item = String>) -> Result<String, String> {
    let mut args = arguments.into_iter();
    let mut values = BTreeMap::new();
    let mut working = false;
    while let Some(key) = args.next() {
        if key == "--working-tree" && !working {
            working = true;
            continue;
        }
        if !matches!(key.as_str(), "--repo-root" | "--base-ref") || values.contains_key(&key) {
            return Err(format!("invalid cachebuster option: {key}"));
        }
        let value = args
            .next()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("missing value after {key}"))?;
        values.insert(key, value);
    }
    let root = values.get("--repo-root").ok_or("--repo-root is required")?;
    let base = values
        .get("--base-ref")
        .cloned()
        .or_else(|| std::env::var("PLUGIN_VERSION_BASE_REF").ok())
        .filter(|v| !v.is_empty())
        .ok_or("--base-ref or PLUGIN_VERSION_BASE_REF is required")?;
    verify(Path::new(root), &base, working)?;
    Ok("plugin cachebuster verification passed".into())
}
