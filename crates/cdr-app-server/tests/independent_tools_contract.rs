use std::io::{BufRead, Write};
use std::time::Duration;

use cdr_app_server::{AppServerClient, AppServerConfig};
use serde_json::{Value, json};

const DESKTOP_ENV: &[&str] = &[
    "CODEX_APP_TOOLS_PIPE_PATH",
    "BROWSER_USE_ACCESS_VERIFYING_IDENTITY",
    "CODEX_TASK_WORKSPACE_VERIFYING_IDENTITY",
    "SKY_CUA_NATIVE_PIPE_DIRECTORY",
    "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
    "CODEX_SESSION_ID",
    "CODEX_THREAD_ID",
];

fn fixture_config() -> AppServerConfig {
    let mut config = AppServerConfig::new(std::env::current_exe().unwrap());
    config.arguments = ["--exact", "stdio_fixture", "--ignored", "--nocapture"]
        .map(str::to_owned)
        .to_vec();
    config
        .environment
        .insert("CDR_INDEPENDENT_STDIO_FIXTURE".into(), "1".into());
    config
}

#[test]
fn default_arguments_use_installed_native_tools_without_a_desktop_pipe() {
    let config = AppServerConfig::new("codex.exe");
    assert_eq!(&config.arguments[..2], ["app-server", "--stdio"]);
    if cfg!(windows) {
        assert_eq!(
            &config.arguments[2..],
            ["-c", r#"mcp_servers.node_repl.env.SKY_CUA_NATIVE_PIPE="0""#]
        );
    } else {
        assert_eq!(config.arguments.len(), 2);
    }
}

#[tokio::test]
async fn initialization_advertises_the_supported_form_protocol() {
    let client = AppServerClient::start(fixture_config()).await.unwrap();
    let result = client
        .request("test/initialization", json!({}), Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(result["capabilities"]["experimentalApi"], true);
    assert_eq!(
        result["capabilities"]["mcpServerOpenaiFormElicitation"],
        true
    );
    assert_eq!(result["clientInfo"]["name"], "codex-discord-remote");
    client.close().await.unwrap();
}

#[cfg(windows)]
#[test]
fn inherited_desktop_state_is_removed_without_changing_the_parent_environment() {
    let temp = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "environment_driver", "--ignored", "--nocapture"]);
    command.env("CDR_INDEPENDENT_ENV_DRIVER", "1");
    command.env("CODEX_HOME", temp.path());
    command.env("CDR_INDEPENDENT_KEEP", "preserved");
    for name in DESKTOP_ENV {
        command.env(name, "inherited-desktop-state");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "isolated environment fixture failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "spawned only as an isolated environment fixture"]
async fn environment_driver() {
    if std::env::var_os("CDR_INDEPENDENT_ENV_DRIVER").is_none() {
        return;
    }
    for explicit_override in [false, true] {
        let mut config = fixture_config();
        if explicit_override {
            config.environment.insert(
                "CODEX_APP_TOOLS_PIPE_PATH".into(),
                "explicit-test-override".into(),
            );
        }
        let client = AppServerClient::start(config).await.unwrap();
        let observed = client
            .request("test/environment", json!({}), Duration::from_secs(2))
            .await
            .unwrap();
        for name in DESKTOP_ENV {
            let expected = if explicit_override && *name == "CODEX_APP_TOOLS_PIPE_PATH" {
                json!("explicit-test-override")
            } else {
                Value::Null
            };
            assert_eq!(observed[*name], expected, "{name}");
            assert_eq!(std::env::var(name).unwrap(), "inherited-desktop-state");
        }
        assert_eq!(
            observed["CODEX_HOME"],
            json!(std::env::var("CODEX_HOME").unwrap())
        );
        assert_eq!(observed["CDR_INDEPENDENT_KEEP"], "preserved");
        client.close().await.unwrap();
    }
}

#[test]
#[ignore = "spawned only as an isolated stdio fixture"]
fn stdio_fixture() {
    if std::env::var_os("CDR_INDEPENDENT_STDIO_FIXTURE").is_none() {
        return;
    }
    let mut initialization = Value::Null;
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let message: Value = serde_json::from_str(&line).unwrap();
        let Some(id) = message.get("id") else {
            continue;
        };
        let result = match message["method"].as_str() {
            Some("initialize") => {
                initialization = message["params"].clone();
                json!({})
            }
            Some("test/initialization") => initialization.clone(),
            Some("test/environment") => {
                let mut environment = serde_json::Map::new();
                for name in DESKTOP_ENV
                    .iter()
                    .copied()
                    .chain(["CODEX_HOME", "CDR_INDEPENDENT_KEEP"])
                {
                    environment.insert(name.into(), json!(std::env::var(name).ok()));
                }
                Value::Object(environment)
            }
            _ => json!({}),
        };
        serde_json::to_writer(&mut stdout, &json!({"id":id,"result":result})).unwrap();
        writeln!(stdout).unwrap();
        stdout.flush().unwrap();
    }
}
