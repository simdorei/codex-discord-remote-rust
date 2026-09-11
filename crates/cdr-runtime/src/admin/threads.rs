//! One-shot list/archive entry points; reuse the bot's exact lifecycle guards.
use super::args::Args;
use crate::{
    action_executor::ActionExecutor,
    app_backend::AppServerTurnBackend,
    bridge_state::BridgeState,
    command_plan::CommandAction,
    queue_runner::QueueCoordinator,
    runtime_paths::{RuntimePaths, discover_inputs},
};
use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::{path::Path, sync::Arc, time::Duration};

pub enum ThreadAdminAction {
    List { limit: u32 },
    Archive { thread_id: String },
}

fn validated_action(action: ThreadAdminAction) -> Result<CommandAction, String> {
    match action {
        ThreadAdminAction::List { limit } => Ok(CommandAction::List { limit }),
        ThreadAdminAction::Archive { thread_id } => {
            if uuid::Uuid::parse_str(&thread_id).is_err()
                || thread_id.len() != 36
                || thread_id.trim() != thread_id
            {
                return Err("archive-thread requires the exact UUID from the current list; names and row numbers are not accepted".into());
            }
            Ok(CommandAction::Archive {
                reference: Some(thread_id),
            })
        }
    }
}

pub(super) async fn run(args: &Args, root: &Path) -> Result<String, String> {
    let action = if args.command == "archive-thread" {
        ThreadAdminAction::Archive {
            thread_id: args.value("--thread-id").unwrap_or("").into(),
        }
    } else {
        ThreadAdminAction::List {
            limit: args
                .value("--limit")
                .unwrap_or("50")
                .parse()
                .map_err(|_| "--limit must be a non-negative integer")?,
        }
    };
    let command = validated_action(action)?;
    let environment = super::project_environment(root)?;
    let paths = RuntimePaths::resolve(
        &environment,
        &discover_inputs(&environment, root.into()).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let config = AppServerConfig::new(&paths.codex_exe).with_environment([(
        "CODEX_HOME".into(),
        paths
            .codex_home
            .to_str()
            .ok_or("Codex home must be UTF-8")?
            .into(),
    )]);
    execute_command(&paths, command, config).await
}

pub async fn execute(
    paths: &RuntimePaths,
    action: ThreadAdminAction,
    config: AppServerConfig,
) -> Result<String, String> {
    execute_command(paths, validated_action(action)?, config).await
}

async fn execute_command(
    paths: &RuntimePaths,
    action: CommandAction,
    config: AppServerConfig,
) -> Result<String, String> {
    // Do not create an empty DB, initialize queue recovery, or run a Discord gateway.
    for (label, path) in [
        ("Codex state", &paths.state_db),
        ("mirror store", &paths.mirror_db),
    ] {
        if !path.is_file() {
            return Err(format!("{label} is unavailable: {}", path.display()));
        }
    }
    let server = Arc::new(
        tokio::time::timeout(Duration::from_secs(20), ResidentAppServer::start(config))
            .await
            .map_err(|error| format!("admin app-server startup timed out: {error}"))?
            .map_err(|error| error.to_string())?,
    );
    let backend = Arc::new(AppServerTurnBackend::new(server.clone()));
    let executor = ActionExecutor::new(
        paths.state_db.clone(),
        paths.mirror_db.clone(),
        Arc::new(BridgeState::new(paths.bridge_state.clone())),
        Arc::new(QueueCoordinator::new(paths.mirror_db.clone(), backend)),
    )
    .with_server(server.clone());
    let result = executor
        .execute(action, 0, 0)
        .await
        .map(|result| result.text)
        .map_err(|error| error.to_string());
    let close = tokio::time::timeout(Duration::from_secs(12), server.close())
        .await
        .map_err(|error| format!("admin app-server close timed out: {error}"))
        .and_then(|result| result.map_err(|error| error.to_string()));
    match (result, close) {
        (Ok(text), Ok(())) => Ok(text),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(primary), Err(close)) => {
            Err(format!("{primary}; app-server close also failed: {close}"))
        }
    }
}
