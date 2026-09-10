use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub(super) fn same_normalized_absolute(left: &Path, right: &Path) -> io::Result<bool> {
    Ok(path_key(left)? == path_key(right)?)
}

fn path_key(path: &Path) -> io::Result<String> {
    let normalized = resolve_nearest_existing_parent(&absolute(path)?)?;
    let key = normalized.to_string_lossy().into_owned();
    #[cfg(windows)]
    return Ok(key.replace('/', "\\").to_lowercase());
    #[cfg(not(windows))]
    Ok(key)
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    })
}

fn resolve_nearest_existing_parent(path: &Path) -> io::Result<PathBuf> {
    let mut existing = PathBuf::new();
    let mut missing = Vec::<OsString>::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => existing.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if missing.pop().is_none() {
                    existing = fs::canonicalize(existing.join(".."))?;
                }
            }
            Component::Normal(name) if missing.is_empty() => {
                match fs::canonicalize(existing.join(name)) {
                    Ok(resolved) => existing = resolved,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        missing.push(name.to_owned());
                    }
                    Err(error) => return Err(error),
                }
            }
            Component::Normal(name) => missing.push(name.to_owned()),
        }
    }
    for component in missing {
        existing.push(component);
    }
    Ok(existing)
}

#[cfg(test)]
mod tests {
    use super::same_normalized_absolute;
    use std::path::Path;

    #[test]
    fn nonexistent_dot_and_parent_aliases_compare_equal() {
        assert!(
            same_normalized_absolute(
                Path::new("not-created/sub/../summary.json"),
                Path::new("./not-created/summary.json")
            )
            .unwrap()
        );
    }

    #[test]
    fn existing_directory_alias_resolves_to_the_same_nonexistent_target() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        let nested = real.join("nested");
        let alias = temp.path().join("alias");
        std::fs::create_dir(&real).unwrap();
        std::fs::create_dir(&nested).unwrap();
        if create_directory_alias(&nested, &alias).is_err() {
            return;
        }
        assert!(
            same_normalized_absolute(&nested.join("summary.json"), &alias.join("summary.json"))
                .unwrap()
        );
        assert!(
            same_normalized_absolute(
                &real.join("peer-summary.json"),
                &alias.join("../peer-summary.json")
            )
            .unwrap()
        );
    }

    #[cfg(windows)]
    fn create_directory_alias(target: &Path, alias: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, alias)
    }

    #[cfg(unix)]
    fn create_directory_alias(target: &Path, alias: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, alias)
    }
}
