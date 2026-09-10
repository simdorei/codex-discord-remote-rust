use std::sync::OnceLock;

use cdr_remote_protocol::output::{CommandDescriptor, RiskTier};
use regex::Regex;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::commands::CommandError;
use crate::files::{MAX_FILE_BYTES, ProjectFileAccess};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredCommand {
    pub descriptor: CommandDescriptor,
    pub arguments: Vec<String>,
}

pub fn discover_commands(
    access: &ProjectFileAccess,
) -> Result<Vec<DiscoveredCommand>, CommandError> {
    let mut commands = Vec::new();
    if access.file_exists("package.json")? {
        let raw = access.read_bytes("package.json", MAX_FILE_BYTES)?;
        commands.extend(package_commands(&raw)?);
    }
    commands.extend(standard_commands(access)?);
    Ok(commands)
}

fn package_commands(raw: &[u8]) -> Result<Vec<DiscoveredCommand>, CommandError> {
    let text = std::str::from_utf8(raw).map_err(|_| manifest_error())?;
    let manifest: PackageManifest = serde_json::from_str(text).map_err(|_| manifest_error())?;
    let mut commands = Vec::new();
    for (name, script) in manifest.scripts.unwrap_or_default().0 {
        if !safe_script_name().is_match(&name) {
            continue;
        }
        let safe_arguments = safe_package_arguments(&script);
        commands.push(DiscoveredCommand {
            descriptor: CommandDescriptor {
                command_id: format!("npm:{name}"),
                display: script.clone(),
                source: "package.json".into(),
                risk_tier: safe_arguments
                    .as_ref()
                    .map_or(RiskTier::Destructive, |_| risk_tier(&name, &script)),
            },
            arguments: safe_arguments.unwrap_or_default(),
        });
    }
    Ok(commands)
}

fn standard_commands(access: &ProjectFileAccess) -> Result<Vec<DiscoveredCommand>, CommandError> {
    let mut commands = Vec::new();
    if access.file_exists("Cargo.toml")? {
        commands.push(command(
            "cargo:test",
            "cargo test",
            "Cargo.toml",
            &["cargo", "test"],
        ));
        commands.push(command(
            "cargo:clippy",
            "cargo clippy",
            "Cargo.toml",
            &["cargo", "clippy"],
        ));
    }
    if access.file_exists("go.mod")? {
        commands.push(command(
            "go:test",
            "go test ./...",
            "go.mod",
            &["go", "test", "./..."],
        ));
    }
    if access.file_exists("pubspec.yaml")? {
        commands.push(command(
            "flutter:test",
            "flutter test",
            "pubspec.yaml",
            &["flutter", "test"],
        ));
    }
    if access.file_exists("pyproject.toml")? && access.root().join("tests").is_dir() {
        let arguments = if access.file_exists("uv.lock")? {
            vec!["uv", "run", "pytest"]
        } else if cfg!(windows) {
            vec!["py", "-3", "-m", "pytest"]
        } else {
            vec!["python3", "-m", "pytest"]
        };
        commands.push(command(
            "python:test",
            "pytest",
            "pyproject.toml",
            &arguments,
        ));
    }
    Ok(commands)
}

fn command(id: &str, display: &str, source: &str, arguments: &[&str]) -> DiscoveredCommand {
    DiscoveredCommand {
        descriptor: CommandDescriptor {
            command_id: id.into(),
            display: display.into(),
            source: source.into(),
            risk_tier: RiskTier::Verify,
        },
        arguments: arguments.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn safe_package_arguments(script: &str) -> Option<Vec<String>> {
    let tokens = script.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 2
        || tokens
            .iter()
            .any(|token| !safe_script_token().is_match(token))
        || !tokens[0].eq_ignore_ascii_case("node")
        || tokens[1] != "--test"
    {
        return None;
    }
    Some(
        ["node", "--preserve-symlinks-main"]
            .into_iter()
            .chain(tokens[1..].iter().copied())
            .map(str::to_owned)
            .collect(),
    )
}

fn risk_tier(name: &str, script: &str) -> RiskTier {
    if destructive_text().is_match(script) {
        RiskTier::Destructive
    } else if network_text().is_match(script) {
        RiskTier::Network
    } else if verify_name().is_match(name) {
        RiskTier::Verify
    } else {
        RiskTier::Read
    }
}

fn manifest_error() -> CommandError {
    CommandError::Manifest {
        path: "package.json".into(),
        reason: "package manifest could not be read".into(),
    }
}

#[derive(Deserialize)]
struct PackageManifest {
    #[serde(default)]
    scripts: Option<OrderedScripts>,
}

#[derive(Default)]
struct OrderedScripts(Vec<(String, String)>);

impl<'de> Deserialize<'de> for OrderedScripts {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ScriptsVisitor;
        impl<'de> Visitor<'de> for ScriptsVisitor {
            type Value = OrderedScripts;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a scripts object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some((name, value)) = map.next_entry::<String, serde_json::Value>()? {
                    if let serde_json::Value::String(script) = value {
                        values.push((name, script));
                    }
                }
                Ok(OrderedScripts(values))
            }
        }
        deserializer.deserialize_map(ScriptsVisitor)
    }
}

macro_rules! pattern {
    ($name:ident, $value:literal) => {
        fn $name() -> &'static Regex {
            static VALUE: OnceLock<Regex> = OnceLock::new();
            VALUE.get_or_init(|| Regex::new($value).expect("valid command pattern"))
        }
    };
}

pattern!(safe_script_name, r"^[A-Za-z0-9:_-]+$");
pattern!(safe_script_token, r"^[A-Za-z0-9_./:@=+-]+$");
pattern!(
    verify_name,
    r"(?i)^(test|tests|typecheck|type-check|lint|check|verify|build|compile)$"
);
pattern!(
    network_text,
    r"(?i)\b(curl|wget)\b|\b(npm|pnpm|yarn|bun)\s+(install|add|update)|\bgit\s+(pull|fetch|clone|push)\b"
);
pattern!(
    destructive_text,
    r"(?i)\bsudo\b|\brm\s+-\w*[rf]\w*\b|\bgit\s+(clean|reset)\b|\b(deploy|publish|release|destroy)\b"
);
