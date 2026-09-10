use std::path::Path;
use std::time::Duration;

use thiserror::Error;
use tokio::process::Command;

const INVENTORY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_INVENTORY_BYTES: usize = 1_048_576;

#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("Codex plugin inventory process could not start: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("Codex plugin inventory query timed out after 10 seconds")]
    Timeout,
    #[error("Codex plugin inventory query failed with exit code {code}: {detail}")]
    Exit { code: i32, detail: String },
    #[error("Codex plugin inventory output exceeded 1 MiB")]
    TooLarge,
    #[error("Codex plugin inventory output was not UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

pub async fn read_codex_plugin_inventory(executable: &Path) -> Result<String, InventoryError> {
    let output = tokio::time::timeout(
        INVENTORY_TIMEOUT,
        Command::new(executable)
            .args(["plugin", "list", "--json"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| InventoryError::Timeout)??;
    if output.stdout.len() > MAX_INVENTORY_BYTES || output.stderr.len() > MAX_INVENTORY_BYTES {
        return Err(InventoryError::TooLarge);
    }
    if !output.status.success() {
        let detail = bounded_detail(if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        });
        return Err(InventoryError::Exit {
            code: output.status.code().unwrap_or(-1),
            detail,
        });
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn bounded_detail(value: &[u8]) -> String {
    String::from_utf8_lossy(value)
        .trim()
        .chars()
        .take(1_000)
        .collect()
}
