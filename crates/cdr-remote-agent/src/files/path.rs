use std::path::{Component, Path, PathBuf};

use super::RemoteFileError;

const SENSITIVE_PARTS: &[&str] = &[
    ".aws",
    ".codex-remote-mcp",
    ".env",
    ".git",
    ".npmrc",
    ".ssh",
    "cookies.txt",
    "credentials",
    "id_ed25519",
    "id_rsa",
    "secrets",
    "gcloud",
];

pub fn validate_relative(value: &str) -> Result<PathBuf, RemoteFileError> {
    let path = Path::new(value);
    let windows_absolute = value.starts_with(['/', '\\'])
        || value.as_bytes().get(1) == Some(&b':')
        || value.starts_with("//")
        || value.starts_with(r"\\");
    if value.is_empty()
        || value.contains('\0')
        || windows_absolute
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
                    | Component::CurDir
            )
        })
        || value.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(RemoteFileError::UnsafePath {
            path: value.to_owned(),
            reason: "relative paths without '..' are required".into(),
        });
    }
    if is_sensitive(path) {
        return Err(RemoteFileError::UnsafePath {
            path: value.to_owned(),
            reason: "sensitive paths are not exposed".into(),
        });
    }
    Ok(path.to_path_buf())
}

pub fn relative_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn is_sensitive(path: &Path) -> bool {
    let parts = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(value) => Some(value.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| SENSITIVE_PARTS.contains(&part.as_str()))
    {
        return true;
    }
    let basename = parts.last().map_or("", String::as_str);
    basename == ".env"
        || basename.starts_with(".env.")
        || [".pem", ".key", ".p12", ".pfx", ".keystore"]
            .iter()
            .any(|suffix| basename.ends_with(suffix))
        || basename.starts_with("id_rsa")
        || ["token", "secret", "credential"]
            .iter()
            .any(|needle| basename.contains(needle))
}
