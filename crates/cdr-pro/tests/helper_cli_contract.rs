use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("plugins/codex-discord-remote")
}

#[cfg(windows)]
#[test]
fn native_probe_commands_match_current_binding_contract_without_plugin_data_or_python() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../fixtures/parity/pro_canonical_probe_lexical_windows.json"
    ))
    .unwrap();
    for (command, field) in [
        ("browser-probe-code", "browser_outer"),
        ("connector-probe-code", "connector_outer"),
        ("connector-retry-probe-code", "connector_retry"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cdr-pro-helper"))
            .arg(command)
            .env_clear()
            .env("PLUGIN_ROOT", expected["plugin_root"].as_str().unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected[field].as_str().unwrap()
        );
    }
}

#[test]
fn standalone_hook_keeps_json_protocol_and_no_interpreter_dependency() {
    let data = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cdr-pro-helper"))
        .arg("browser-hook")
        .env_clear()
        .env("PLUGIN_ROOT", root())
        .env("PLUGIN_DATA", data.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = json!({"hook_event_name":"Stop","last_assistant_message":"Chrome is unavailable.","session_id":"a","turn_id":"b"});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["decision"], "block");
}

#[test]
fn installed_hook_manifests_reference_native_helpers_only() {
    for name in ["browser-evidence.json", "pro-connector-evidence.json"] {
        let value: Value = serde_json::from_str(
            &std::fs::read_to_string(root().join("hooks").join(name)).unwrap(),
        )
        .unwrap();
        for groups in value["hooks"].as_object().unwrap().values() {
            for group in groups.as_array().unwrap() {
                for hook in group["hooks"].as_array().unwrap() {
                    assert!(
                        hook["command"]
                            .as_str()
                            .unwrap()
                            .starts_with("\"${PLUGIN_ROOT}/bin/cdr-pro-helper\" ")
                    );
                    assert!(
                        hook["commandWindows"]
                            .as_str()
                            .unwrap()
                            .starts_with("\"${PLUGIN_ROOT}\\bin\\cdr-pro-helper.exe\" ")
                    );
                }
            }
        }
    }
}
