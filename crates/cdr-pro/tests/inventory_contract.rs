use std::path::Path;

use cdr_pro::inventory::{InventoryError, read_codex_plugin_inventory};

#[tokio::test]
async fn missing_codex_executable_surfaces_the_actual_spawn_failure() {
    let error = read_codex_plugin_inventory(Path::new(
        "definitely-missing-codex-executable-for-contract-test.exe",
    ))
    .await
    .unwrap_err();

    assert!(matches!(error, InventoryError::Spawn(_)));
    assert!(error.to_string().contains("could not start"));
}
