use std::{path::Path, process::Command};

use cdr_pro::evidence::{canonical_browser_inner_probe_code, canonical_connector_inner_probe_code};
use serde_json::Value;

fn plugin_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/codex-discord-remote")
        .canonicalize()
        .unwrap()
}

fn node(source: &str) -> Value {
    let output = Command::new("node")
        .args(["--input-type=module", "--eval", source])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn browser_probe_reuses_lexical_chrome_without_selecting_another_browser() {
    let code = canonical_browser_inner_probe_code(&plugin_root()).unwrap();
    let output = node(&format!(
        r"
let selections = 0;
let lists = 0;
let evidence;
const agent = {{ browsers: {{ get: async () => {{ selections++; throw Error('unexpected acquisition'); }} }} }};
const chrome = {{ tabs: {{ list: async () => {{ lists++; return []; }} }} }};
const nodeRepl = {{ write: value => {{ evidence = JSON.parse(value); }} }};
{code}
process.stdout.write(JSON.stringify({{ evidence, selections, lists }}));
"
    ));
    assert_eq!(output["evidence"]["status"], "available");
    assert_eq!(output["evidence"]["selected_stage"], "existing_binding");
    assert_eq!(output["selections"], 0);
    assert_eq!(output["lists"], 1);
}

#[test]
fn connector_probe_receives_lexical_tab_and_preserves_the_origin_guard() {
    let code = canonical_connector_inner_probe_code(&plugin_root()).unwrap();
    let output = node(&format!(
        r"
let reads = 0;
let evidence;
const proConversationTab = {{ playwright: {{}}, url: async () => {{ reads++; return 'https://example.invalid/'; }} }};
const nodeRepl = {{ write: value => {{ evidence = JSON.parse(value); }} }};
{code}
process.stdout.write(JSON.stringify({{ evidence, reads }}));
"
    ));
    assert_eq!(output["reads"], 1);
    assert_eq!(output["evidence"]["status"], "failed");
    assert_eq!(output["evidence"]["failed_stage"], "conversation_url");
}
