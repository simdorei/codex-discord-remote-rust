use cdr_runtime::soak::{SoakConfig, run};

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("offline_soak_error: {error}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = SoakConfig::from_env()?;
    let summary = run(&config).await?;
    println!("{}", serde_json::to_string(&summary)?);
    if summary.status != "passed" {
        std::process::exit(2);
    }
    Ok(())
}
