use cdr_runtime::soak::{SoakConfig, run};

#[path = "../../tests/fixtures/app_server.rs"]
mod app_server;
#[path = "../../tests/fixtures/codex_cli.rs"]
mod codex_cli;

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("offline_soak_error: {error}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(root) = std::env::var_os("CDR_CODEX_CLI_FIXTURE_ROOT") {
        return codex_cli::run(std::path::Path::new(&root));
    }
    if std::env::args().nth(1).as_deref() == Some("--app-server-fixture") {
        return app_server::run();
    }
    let config = SoakConfig::from_env()?;
    let summary = run(&config).await?;
    println!("{}", serde_json::to_string(&summary)?);
    if summary.status != "passed" {
        std::process::exit(2);
    }
    Ok(())
}
