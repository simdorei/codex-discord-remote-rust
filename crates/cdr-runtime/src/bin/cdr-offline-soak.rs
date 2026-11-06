use cdr_runtime::soak::{SoakConfig, run};

#[path = "../../tests/fixtures/app_server.rs"]
mod app_server;
#[path = "../../tests/fixtures/codex_cli.rs"]
mod codex_cli;
#[path = "../../tests/fixtures/version_probe.rs"]
mod version_probe;

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("offline_soak_error: {error}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if std::env::args().any(|arg| arg.contains("Get-AppxPackage"))
        && let Some(packages) = std::env::var_os("CDR_FIXTURE_STORE_PACKAGES")
    {
        println!("{}", packages.to_string_lossy());
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--version") {
        version_probe::before_version()?;
        let version = std::env::current_exe()?.with_file_name("probe-version.txt");
        let version = if version.exists() {
            std::fs::read_to_string(version)?
        } else {
            "0.166.0-alpha.6".to_owned()
        };
        println!("codex-cli {}", version.trim());
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--probe-child") {
        std::thread::sleep(std::time::Duration::from_mins(1));
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("app-server") {
        return app_server::run_standalone_double();
    }
    if let Some(root) = std::env::var_os("CDR_CODEX_CLI_FIXTURE_ROOT") {
        return codex_cli::run(std::path::Path::new(&root));
    }
    if std::env::args().nth(1).as_deref() == Some("plugin") {
        println!("[]");
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("init-state-db") {
        let db_path = std::env::args()
            .nth(2)
            .ok_or("init-state-db requires <path>")?;
        if let Some(parent) = std::path::Path::new(&db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                cwd TEXT NOT NULL DEFAULT '',
                updated_at INTEGER NOT NULL DEFAULT 0,
                rollout_path TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                reasoning_effort TEXT NOT NULL DEFAULT '',
                tokens_used INTEGER NOT NULL DEFAULT 0,
                archived INTEGER NOT NULL DEFAULT 0,
                archived_at INTEGER,
                source TEXT NOT NULL DEFAULT 'vscode',
                thread_source TEXT NOT NULL DEFAULT 'user'
            );",
        )?;
        return Ok(());
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
