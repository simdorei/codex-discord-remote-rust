use cdr_app_server::{USED_CLIENT_REQUESTS, USED_NOTIFICATIONS, USED_SERVER_REQUESTS};
use serde_json::Value;

#[test]
fn rust_method_inventory_matches_python_plus_explicit_archive_scope_extension() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/parity/app_server_contract.json"
    ))
    .expect("app-server contract fixture");
    // Preserve the historical Python capture; thread/list was added in Rust
    // to verify the descendant scope before archiving the original thread.
    let mut expected_clients = fixture["client_requests"].as_array().unwrap().clone();
    assert!(!expected_clients.contains(&Value::from("thread/list")));
    expected_clients.push(Value::from("thread/list"));
    expected_clients.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    assert_eq!(
        Value::Array(expected_clients),
        serde_json::to_value(USED_CLIENT_REQUESTS).expect("client methods")
    );
    assert_eq!(
        fixture["server_requests"],
        serde_json::to_value(USED_SERVER_REQUESTS).expect("server methods")
    );
    assert_eq!(
        fixture["notifications"],
        serde_json::to_value(USED_NOTIFICATIONS).expect("notifications")
    );
    assert_eq!(fixture["initialization"]["experimentalApi"], true);
    assert_eq!(fixture["codex_version"], "0.146.0-alpha.9.2");
}
