use super::args::Args;
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant, UNIX_EPOCH};

pub(super) fn run(args: &Args, root: &Path) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    if args.command == "codex-version" {
        let mut arguments = vec![
            "--admin".into(),
            "codex-version-worker".into(),
            "--repo-root".into(),
            root.display().to_string(),
        ];
        for key in ["--pid", "--identity", "--bot-identity"] {
            if let Some(value) = args.value(key) {
                arguments.extend([key.into(), value.into()]);
            }
        }
        let result = cdr_windows_native::capture_bounded(
            &std::env::current_exe().map_err(|e| e.to_string())?,
            &arguments,
            root,
            &std::env::vars().collect(),
            deadline,
            Duration::from_secs(1),
            65_536,
        )
        .map_err(|e| e.to_string())?;
        if result.code != 0 {
            return Err(String::from_utf8_lossy(&result.stderr).trim().into());
        }
        return String::from_utf8(result.stdout)
            .map(|text| text.trim().into())
            .map_err(|e| e.to_string());
    }
    observe(args, root, deadline)
}

fn observe(args: &Args, root: &Path, deadline: Instant) -> Result<String, String> {
    let pid = args
        .required("--pid")?
        .parse::<u32>()
        .map_err(|_| "invalid process PID")?;
    let expected = args.required("--identity")?;
    let observe = || -> Result<cdr_windows_native::LiveProcessMetadata, String> {
        let process = cdr_windows_native::query_live_process_metadata(pid)
            .map_err(|e| e.to_string())?
            .ok_or("process is not running")?;
        if process.process_identity != expected {
            return Err("process identity changed".into());
        }
        Ok(process)
    };
    let process = observe()?;
    let version = if let Some(bot_identity) = args.value("--bot-identity") {
        let bot_pid = bot_identity
            .split('|')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .ok_or("invalid bot identity")?;
        let bot = cdr_windows_native::query_live_process_metadata(bot_pid)
            .map_err(|e| e.to_string())?
            .ok_or("bot is not running")?;
        if bot.process_identity != bot_identity {
            return Err("bot identity changed".into());
        }
        let mut bytes = vec![];
        std::fs::File::open(root.join(".codex_discord_rust.codex-selection.json"))
            .map_err(|e| e.to_string())?
            .take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65_536 {
            return Err("selection receipt too large".into());
        }
        let receipt: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if receipt["schema_version"] != 1
            || receipt["runtime_process_identity"] != bot_identity
            || receipt["app_server_process_identity"] != expected
            || receipt["initialize_completed"] != true
        {
            return Err("current initialized selection not observed".into());
        }
        let selected: crate::runtime_paths::selection::CodexSelection =
            serde_json::from_value(receipt["selection"].clone()).map_err(|e| e.to_string())?;
        if std::fs::canonicalize(&selected.path).map_err(|e| e.to_string())?
            != std::fs::canonicalize(&process.executable_path).map_err(|e| e.to_string())?
        {
            return Err("selection image changed".into());
        }
        let _lease = selected
            .lease(
                deadline
                    .checked_sub(Duration::from_secs(1))
                    .unwrap_or_else(Instant::now),
            )
            .map_err(|e| e.to_string())?;
        selected.version
    } else {
        let modified = std::fs::metadata(&process.executable_path)
            .and_then(|m| m.modified())
            .map_err(|e| e.to_string())?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        let modified_ticks = modified.as_nanos() / 100 + 621_355_968_000_000_000;
        let creation_ticks = expected
            .split('|')
            .nth(1)
            .and_then(|s| s.parse::<u128>().ok())
            .ok_or("invalid process start identity")?;
        if modified_ticks > creation_ticks {
            return Err("file changed; restart to verify".into());
        }
        crate::runtime_paths::selection::probe_version(
            &process.executable_path,
            deadline
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
        )
        .map_err(|e| e.to_string())?
    };
    if observe()? != process {
        return Err("process observation changed".into());
    }
    Ok(version)
}
