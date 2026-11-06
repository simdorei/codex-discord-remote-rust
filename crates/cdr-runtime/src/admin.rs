//! One-shot installation helpers. These commands never start the Discord gateway.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod args;
mod async_recovery;
pub mod attachment;
pub(crate) mod codex_tools;
#[cfg(windows)]
mod codex_version;
pub mod env_file;
mod first_reply;
mod manual_reserve;
mod recovery_capability;
pub mod setup;
mod store;
pub mod threads;
mod writer_recovery;

use args::Args;

pub async fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<String, String> {
    let args = Args::parse(arguments)?;
    let root = args.root()?;
    match args.command.as_str() {
        "setup-discord" => Box::pin(setup::run(&args, &root)).await,
        "send-attachment" => Box::pin(attachment::run(&args, &root)).await,
        "list-threads" | "archive-thread" => Box::pin(threads::run(&args, &root)).await,
        "configure-install" => configure_install(&args, &root),
        "discover-codex" => discover_codex(&args, &root),
        "diagnose-codex-tools" => codex_tools::run(&args, &root).await,
        #[cfg(windows)]
        "codex-selection-worker" => {
            match (args.value("--request-file"), args.value("--record-json")) {
                (Some(_), None) => crate::runtime_paths::selection_worker::run(Path::new(
                    args.required("--request-file")?,
                )),
                (None, Some(record)) => crate::runtime_paths::receipt::persist(&root, record)
                    .map_err(|error| error.to_string()),
                _ => {
                    Err("selection worker requires exactly one request-file or record-json".into())
                }
            }
        }
        #[cfg(windows)]
        "codex-selected-command" => {
            let selection = serde_json::from_str(args.required("--selection-json")?)
                .map_err(|e| format!("invalid selection: {e}"))?;
            let arguments = serde_json::from_str(args.required("--arguments-json")?)
                .map_err(|e| format!("invalid command arguments: {e}"))?;
            let output = crate::runtime_paths::selection_worker::execute_selected_using(
                &std::env::current_exe().map_err(|e| e.to_string())?,
                selection,
                arguments,
                PathBuf::from(args.required("--codex-home")?),
                &root,
                std::time::Instant::now() + std::time::Duration::from_secs(115),
            )
            .map_err(|e| e.to_string())?;
            serde_json::to_string(&output).map_err(|e| e.to_string())
        }
        #[cfg(windows)]
        "codex-version" | "codex-version-worker" => codex_version::run(&args, &root),
        "verify-plugin-inventory" => verify_inventory(&args, &root),
        "retire-automatic-reserve" | "authorize-saved-final" => manual_reserve::run(&args, &root),
        "backup-store" => store::backup(&root),
        "active-queue-count" => store::active_count(&root),
        "recover-writer" | "recover-tools" => Box::pin(writer_recovery::run(&args, &root)).await,
        "inspect-new-first-reply" => first_reply::run(&args, &root),
        "inspect-async-recovery" => async_recovery::run(&args, &root),
        "check-recovery-compatibility" => recovery_capability::run(&args, &root),
        unknown => Err(format!("unknown admin command: {unknown}")),
    }
}

fn configure_install(args: &Args, root: &Path) -> Result<String, String> {
    let codex_home = args.required("--codex-home")?;
    let mut updates = vec![("CODEX_HOME", codex_home)];
    if let Some(executable) = args.value("--codex-exe") {
        updates.push(("CODEX_EXE", executable));
    }
    env_file::update(&root.join(".env"), &updates)?;
    Ok("Configured installation environment; existing unrelated settings preserved.".into())
}

fn project_environment(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut environment = std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    crate::startup::merge_env_file(&mut environment, &root.join(".env"))
        .map_err(|e| e.to_string())?;
    Ok(environment)
}

fn discover_codex(args: &Args, root: &Path) -> Result<String, String> {
    let mut environment = project_environment(root)?;
    if let Some(exe) = args.value("--codex-exe") {
        environment.insert("CODEX_EXE".into(), exe.into());
    }
    if let Some(home) = args.value("--codex-home") {
        environment.insert("CODEX_HOME".into(), home.into());
    }
    let paths = crate::runtime_paths::RuntimePaths::discover(
        &environment,
        root,
        std::time::Instant::now() + std::time::Duration::from_secs(15),
        std::time::Duration::from_secs(3),
    )
    .map_err(|e| e.to_string())?;
    if let Some(format) = args.value("--format") {
        if format != "json" {
            return Err("discover-codex --format must be json".into());
        }
        #[cfg(windows)]
        return serde_json::to_string(&paths.codex_selection).map_err(|e| e.to_string());
        #[cfg(not(windows))]
        return serde_json::to_string(
            &serde_json::json!({"path":paths.codex_exe,"source":paths.codex_exe_source}),
        )
        .map_err(|e| e.to_string());
    }
    Ok(paths.codex_exe.display().to_string())
}

fn verify_inventory(args: &Args, root: &Path) -> Result<String, String> {
    let path = |key| -> Result<PathBuf, String> { Ok(root.join(args.required(key)?)) };
    let version = cdr_pro::installation::verify_inventory(
        &path("--marketplace-inventory")?,
        &path("--plugin-inventory")?,
        &path("--plugin-manifest")?,
        root,
        args.value("--marketplace-name")
            .unwrap_or("codex-discord-remote"),
        args.value("--plugin-id")
            .unwrap_or("codex-discord-remote@codex-discord-remote"),
    )
    .map_err(|error| format!("INSTALL_INCOMPLETE: {error}"))?;
    Ok(format!("Verified Codex plugin inventory: {version}"))
}
