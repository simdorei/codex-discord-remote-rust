use std::fs;
use std::path::Path;

use cdr_pro::fingerprint::{fingerprint_required_plugins, tree_digest};
use serde_json::{Value, json};

fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/pro-fingerprint")
}

#[test]
fn plugin_tree_hashes_match_frozen_python_results() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../fixtures/parity/pro_plugin_tree_hashes.json"
    ))
    .expect("parse frozen tree hashes");
    let root = fixture_root();
    assert_eq!(
        tree_digest(&root.join("remote")).expect("hash remote fixture"),
        expected["remote"].as_str().expect("remote hash fixture")
    );
    assert_eq!(
        tree_digest(&root.join("chrome")).expect("hash chrome fixture"),
        expected["chrome"].as_str().expect("chrome hash fixture")
    );
}

#[test]
fn required_plugin_fingerprint_is_stable_and_changes_with_content() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let remote = temp.path().join("remote");
    let chrome = temp.path().join("chrome");
    fs::create_dir_all(&remote).expect("create remote fixture");
    fs::create_dir_all(&chrome).expect("create chrome fixture");
    fs::write(remote.join("plugin.json"), "remote").expect("write remote fixture");
    fs::write(chrome.join("plugin.json"), "chrome").expect("write chrome fixture");
    let inventory = || {
        json!({
            "installed": [
                {"pluginId": "codex-discord-remote@codex-discord-remote", "installed": true, "enabled": true, "version": "1", "source": {"path": remote}},
                {"pluginId": "chrome@openai-bundled", "installed": true, "enabled": true, "version": "2", "source": {"path": chrome}}
            ]
        })
        .to_string()
    };
    let first = fingerprint_required_plugins(&inventory()).expect("first fingerprint");
    assert_eq!(
        fingerprint_required_plugins(&inventory()).expect("stable fingerprint"),
        first
    );
    fs::write(remote.join("plugin.json"), "remote changed").expect("change remote fixture");
    assert_ne!(
        fingerprint_required_plugins(&inventory()).expect("changed fingerprint"),
        first
    );
}
