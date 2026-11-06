use super::args::Args;
use std::path::Path;

pub(super) async fn run(args: &Args, root: &Path) -> Result<String, String> {
    let environment = super::project_environment(root)?;
    let paths = crate::runtime_paths::RuntimePaths::discover(
        &environment,
        root,
        std::time::Instant::now() + std::time::Duration::from_secs(15),
        std::time::Duration::from_secs(3),
    )
    .map_err(|e| e.to_string())?;
    let thread = args.required("--thread-id")?;
    if cdr_codex_state::CodexThreadStore::open(&paths.state_db)
        .map_err(|e| e.to_string())?
        .load_thread(thread, false)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Err("Recovery target is not an existing active thread".into());
    }
    let target = crate::writer_recovery::Target {
        root,
        codex_home: &paths.codex_home,
        database: &paths.mirror_db,
        thread,
        channel: args
            .required("--channel-id")?
            .parse()
            .map_err(|_| "invalid channel ID")?,
        user: args
            .required("--owner-user-id")?
            .parse()
            .map_err(|_| "invalid owner user ID")?,
    };
    let report = if args.command == "recover-tools" {
        crate::writer_recovery::run_tools(target, args.dry_run).await?
    } else {
        crate::writer_recovery::run(target, args.dry_run, false).await?
    };
    serde_json::to_string(&report).map_err(|e| e.to_string())
}
