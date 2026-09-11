use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use cdr_pro::hooks::{self, HookKind};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("collect-release-evidence") {
        return cdr_pro::release_collection::run_cli(args);
    }
    if command.as_deref() == Some("verify-cachebuster") {
        println!("{}", cdr_pro::cachebuster::run(args)?);
        return Ok(());
    }
    if command.as_deref() == Some("conversation") {
        println!("{}", cdr_pro::conversation::run(args)?);
        return Ok(());
    }
    let root = plugin_root()?;
    if matches!(
        command.as_deref(),
        Some("browser-probe-code" | "connector-probe-code" | "connector-retry-probe-code")
    ) {
        if args.next().is_some() {
            return Err("unexpected probe argument".into());
        }
        let code = match command.as_deref() {
            Some("browser-probe-code") => cdr_pro::evidence::canonical_browser_probe_code(&root),
            Some("connector-probe-code") => {
                cdr_pro::evidence::canonical_connector_probe_code(&root)
            }
            _ => cdr_pro::evidence::canonical_connector_retry_probe_code(&root),
        }
        .map_err(|e| e.to_string())?;
        print!("{code}");
        return Ok(());
    }
    let kind = match command.as_deref() {
        Some("browser-hook") => HookKind::Browser,
        Some("connector-hook") => HookKind::Connector,
        _ => return Err("usage: cdr-pro-helper browser-hook|connector-hook".into()),
    };
    if args.next().is_some() {
        return Err("unexpected helper argument".into());
    }
    let data = std::env::var_os("PLUGIN_DATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or("evidence_hook_failed stage=plugin_data_missing")?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("evidence hook input exceeded 8 MiB".into());
    }
    let payload =
        serde_json::from_slice(&bytes).map_err(|_| "evidence hook input was not valid JSON")?;
    if let Some(output) = hooks::handle(kind, &payload, &root, &data)? {
        println!("{output}");
    }
    Ok(())
}

fn plugin_root() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("PLUGIN_ROOT").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(value));
    }
    Ok(std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("could not locate plugin root")?
        .to_path_buf())
}
