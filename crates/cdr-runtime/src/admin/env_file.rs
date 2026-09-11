//! Preserve unrelated settings and publish all validated updates atomically.

use std::fs;
use std::io::Write;
use std::path::Path;

pub fn read(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.trim_start_matches('\u{feff}').to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("could not read environment file: {error}")),
    }
}

#[must_use]
pub fn get<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == name).then(|| value.trim().trim_matches('"').trim_matches('\''))
        })
}

pub fn update(path: &Path, updates: &[(&str, &str)]) -> Result<(), String> {
    for (name, value) in updates {
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err("invalid environment setting name".into());
        }
        if value.contains(['\n', '\r', '\0']) {
            return Err(format!("{name} cannot contain a newline or NUL"));
        }
    }
    let text = read(path)?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    for (name, value) in updates {
        let mut found = false;
        for line in &mut lines {
            if !line.trim_start().starts_with('#')
                && line
                    .split_once('=')
                    .is_some_and(|(key, _)| key.trim() == *name)
            {
                *line = format!("{name}={value}");
                found = true;
            }
        }
        if !found {
            lines.push(format!("{name}={value}"));
        }
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    if path.exists() {
        #[cfg(windows)]
        cdr_windows_native::copy_file_access_rules(path, staged.path())
            .map_err(|e| format!("could not preserve environment file access rules: {e}"))?;
        staged
            .as_file()
            .set_permissions(fs::metadata(path).map_err(|e| e.to_string())?.permissions())
            .map_err(|e| e.to_string())?;
    }
    write!(staged, "{}{newline}", lines.join(newline)).map_err(|e| e.to_string())?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    staged
        .persist(path)
        .map_err(|e| format!("could not publish environment file: {}", e.error))?;
    Ok(())
}
