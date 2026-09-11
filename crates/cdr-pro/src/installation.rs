//! Verify the CLI's installation receipt against the requested repository and version.

use std::path::Path;

use serde_json::Value;

fn read_object(path: &Path, label: &str) -> Result<Value, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("{label} could not be read: {e}"))?;
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("{label} is not valid JSON: {e}"))?;
    if !value.is_object() {
        return Err(format!("{label} must be a JSON object"));
    }
    Ok(value)
}

fn required<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{field} must be a nonempty string"))
}

fn single<'a>(
    value: &'a Value,
    collection: &str,
    field: &str,
    expected: &str,
) -> Result<&'a Value, String> {
    let records = value
        .get(collection)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{collection} must be a JSON array"))?;
    if records.iter().any(|record| !record.is_object()) {
        return Err(format!("{collection} entries must be JSON objects"));
    }
    let matching: Vec<_> = records
        .iter()
        .filter(|record| record.get(field).and_then(Value::as_str) == Some(expected))
        .collect();
    if matching.len() != 1 {
        return Err(format!(
            "{collection} '{expected}' was not registered exactly once"
        ));
    }
    Ok(matching[0])
}

pub fn verify_inventory(
    marketplaces: &Path,
    plugins: &Path,
    manifest: &Path,
    expected_root: &Path,
    marketplace_name: &str,
    plugin_id: &str,
) -> Result<String, String> {
    let marketplaces = read_object(marketplaces, "marketplace inventory")?;
    let plugins = read_object(plugins, "plugin inventory")?;
    let manifest = read_object(manifest, "plugin manifest")?;
    let version = required(&manifest, "version")?;
    let marketplace = single(&marketplaces, "marketplaces", "name", marketplace_name)?;
    let actual_root = Path::new(required(marketplace, "root")?)
        .canonicalize()
        .map_err(|e| format!("marketplace root could not be resolved: {e}"))?;
    let expected_root = expected_root
        .canonicalize()
        .map_err(|e| format!("expected root could not be resolved: {e}"))?;
    if !same_path(&actual_root, &expected_root) {
        return Err(format!(
            "marketplace '{marketplace_name}' points to the wrong repository"
        ));
    }
    let plugin = single(&plugins, "installed", "pluginId", plugin_id)?;
    if plugin.get("installed").and_then(Value::as_bool) != Some(true) {
        return Err(format!("plugin '{plugin_id}' is not installed"));
    }
    if plugin.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Err(format!("plugin '{plugin_id}' is not enabled"));
    }
    let actual_version = required(plugin, "version")?;
    if actual_version != version {
        return Err(format!(
            "plugin '{plugin_id}' version mismatch: expected '{version}', got '{actual_version}'"
        ));
    }
    Ok(version.into())
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}
