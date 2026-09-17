//! Opt-in, read-only account capability probe. Never starts or resumes a thread.
use cdr_app_server::{AppServerClient, AppServerConfig};
use serde_json::{Value, json};
use std::time::Duration;

type ProbeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> ProbeResult<()> {
    let executable = std::env::var_os("CDR_LIVE_CODEX_EXE")
        .ok_or("CDR_LIVE_CODEX_EXE must identify the installed Codex executable")?;
    let mut config = AppServerConfig::new(executable);
    config.client_name = "cdr_luna_reserve_readonly_probe".into();
    config.client_title = "Discord Remote Reserve capability probe".into();
    let client = AppServerClient::start(config).await?;
    let result = tokio::time::timeout(Duration::from_secs(45), probe(&client)).await;
    client.close().await?;
    println!("probe_client_closed=true; thread_start_count=0; turn_start_count=0");
    result??;
    Ok(())
}

async fn probe(client: &AppServerClient) -> ProbeResult<()> {
    // Explicit selection does not implement automatic fallback. Do not advertise
    // supportsLunaReserve, whose official contract is for automatic-fallback clients.
    let value = client
        .request(
            "account/rateLimits/read",
            json!({}),
            Duration::from_secs(15),
        )
        .await?;
    let mut safe = serde_json::Map::new();
    for key in ["ordinaryUsageAllowed", "rateLimits", "rateLimitsByLimitId"] {
        if let Some(field) = value.get(key) {
            safe.insert(key.into(), field.clone());
        }
    }
    println!(
        "{}",
        json!({"probe":"explicit_reserve_capability","response":safe})
    );
    let catalog = client
        .request("model/list", json!({}), Duration::from_secs(10))
        .await?;
    let rows: Vec<_> = catalog.get("data").and_then(Value::as_array).into_iter().flatten()
        .filter(|row| ["model", "id", "displayName"].iter().any(|key| {
            row.get(*key).and_then(Value::as_str).is_some_and(|s| {
                let lower = s.to_ascii_lowercase();
                lower.contains("luna") || lower.contains("reserve")
            })
        }))
        .map(|row| json!({"model":row.get("model"),"id":row.get("id"),"displayName":row.get("displayName"),"hidden":row.get("hidden"),"defaultReasoningEffort":row.get("defaultReasoningEffort"),"supportedReasoningEfforts":row.get("supportedReasoningEfforts")}))
        .collect();
    println!("{}", json!({"probe":"model_catalog","rows":rows}));
    Ok(())
}
