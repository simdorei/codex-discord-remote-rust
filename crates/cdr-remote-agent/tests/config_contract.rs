use std::collections::HashMap;

use cdr_remote_agent::config::{ConfigError, load_remote_mcp_config};

fn environment(url: &str) -> HashMap<String, String> {
    HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "true".into()),
        ("CODEX_REMOTE_MCP_BRIDGE_URL".into(), url.into()),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "device-a".into()),
        (
            "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
            "device-secret-never-print".into(),
        ),
    ])
}

#[test]
fn cfg1_disabled_is_none_and_secure_or_exact_loopback_urls_pass() {
    assert_eq!(
        load_remote_mcp_config(&HashMap::new()).expect("disabled"),
        None
    );
    for url in [
        "ws://localhost:8030/bridge",
        "ws://127.0.0.1:8030/bridge",
        "ws://[::1]:8030/bridge",
        "wss://simdorei.duckdns.org/bridge",
    ] {
        let config = load_remote_mcp_config(&environment(url))
            .expect("valid config")
            .expect("enabled");
        assert_eq!(config.bridge_url.as_str(), url);
        assert_eq!(config.binding_ttl_seconds, 1_800);
        assert!(!format!("{config:?}").contains("device-secret-never-print"));
    }
}

#[test]
fn cfg2_deceptive_plaintext_fragments_credentials_and_bad_ttl_fail_closed() {
    for url in [
        "ws://localhost@evil.example/bridge",
        "ws://localhost.evil.example/bridge",
        "ws://127.0.0.1.evil.example/bridge",
        "ws://[::1]@evil.example/bridge",
        "wss://example.test/bridge#ignored",
        "ws://localhost:8030/bridge#ignored",
    ] {
        assert!(matches!(
            load_remote_mcp_config(&environment(url)),
            Err(ConfigError::InvalidBridgeUrl)
        ));
    }
    let mut values = environment("wss://example.test/bridge");
    values.insert("CODEX_REMOTE_MCP_BINDING_TTL_SECONDS".into(), "59".into());
    assert!(matches!(
        load_remote_mcp_config(&values),
        Err(ConfigError::InvalidInteger { .. })
    ));
}

#[test]
fn cfg3_missing_required_values_never_leak_token_material() {
    let mut values = environment("wss://example.test/bridge");
    values.insert("CODEX_REMOTE_MCP_DEVICE_ID".into(), String::new());
    let error = load_remote_mcp_config(&values).expect_err("device id required");
    let text = error.to_string();
    assert!(text.contains("CODEX_REMOTE_MCP_DEVICE_ID"));
    assert!(!text.contains("device-secret-never-print"));
}
