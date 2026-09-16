use std::collections::BTreeSet;
use std::sync::Arc;

use cdr_app_server::{AppServerError, ResidentAppServer};
use cdr_codex_state::CodexThreadStore;
use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_remote_agent::status::RemoteAgentStatus;
use cdr_store::StoreError;
use cdr_store::mapping::{mirror_targets, remaining_discord_ids};
use cdr_store::queue::completed_app_server_fork_target_for_source;
use cdr_store::schema::open_initialized;

use super::DiscordRuntimeError;
use crate::action_executor::ActionExecutor;
use crate::app_backend::AppServerTurnBackend;
use crate::archive_delete::ArchiveDeletePaths;
use crate::bridge_state::BridgeState;
use crate::config::RuntimeConfig;
use crate::discord_dispatch::AutocompleteCatalog;
use crate::pro_runtime::ProPromptRuntime;
use crate::queue_recovery_transport::stabilize_after_queue_recovery;
use crate::queue_runner::QueueCoordinator;
use crate::reserve_auto::ReserveAutoController;
use crate::restart_readiness::drain::AdmissionGate;
use crate::runtime_paths::RuntimePaths;

pub(super) async fn build_executor(
    paths: &RuntimePaths,
    config: &RuntimeConfig,
    server: Arc<ResidentAppServer>,
    remote_status: RemoteAgentStatus,
    admission: AdmissionGate,
) -> Result<
    (
        Arc<QueueCoordinator<AppServerTurnBackend>>,
        Arc<ActionExecutor<AppServerTurnBackend>>,
    ),
    DiscordRuntimeError,
> {
    let reserve_auto = ReserveAutoController::new(Arc::clone(&server), paths.mirror_db.clone());
    let backend = Arc::new(
        AppServerTurnBackend::new(Arc::clone(&server))
            .with_timeouts(
                config.app_server_resume_timeout,
                config.app_server_history_read_timeout,
            )
            .with_pro_skill_path(
                paths
                    .root
                    .join("plugins/codex-discord-remote/skills/ask-chatgpt-pro/SKILL.md"),
            )
            .with_reserve_auto(Arc::clone(&reserve_auto)),
    );
    let queue = Arc::new(QueueCoordinator::new_with_admission_gate(
        paths.mirror_db.clone(),
        backend,
        admission,
    ));
    let bridge_state = Arc::new(BridgeState::new(paths.bridge_state.clone()));
    let recovery = queue.recover().await;
    // Exact-ID routing must not replay historical ownership transfers on startup.
    let reconciliation =
        if crate::queue_runner::TurnBackend::requires_app_server_fork(queue.backend.as_ref()) {
            reconcile_bridge_state(bridge_state.as_ref(), &paths.mirror_db)
        } else {
            Ok(())
        };
    let recovery = match recovery {
        Ok(recovery) => {
            reconciliation?;
            recovery
        }
        Err(error) => {
            if let Err(reconciliation) = reconciliation {
                eprintln!(
                    "secondary_bridge_state_reconciliation_error_after_queue_recovery: {reconciliation}"
                );
            }
            return Err(error.into());
        }
    };
    eprintln!("rust_queue_recovery: {recovery:?}");
    if stabilize_after_queue_recovery(server.as_ref()).await? {
        eprintln!(
            "rust_queue_recovery_app_server_restarted generation={}",
            server.generation()
        );
    }
    let pro = Arc::new(
        ProPromptRuntime::capture(
            paths.root.clone(),
            paths.state_db.clone(),
            paths.codex_exe.clone(),
            Arc::clone(&server),
            config.remote_mcp.clone(),
            remote_status,
        )
        .await,
    );
    let executor = Arc::new(
        ActionExecutor::new(
            paths.state_db.clone(),
            paths.mirror_db.clone(),
            bridge_state,
            Arc::clone(&queue),
        )
        .with_server(server)
        .with_reserve_auto(reserve_auto)
        .with_app_server_resume_timeout(config.app_server_resume_timeout)
        .with_archive_delete_paths(archive_delete_paths(paths))
        .with_host_commands(config.host_commands)
        .with_prompt_preprocessor(pro),
    );
    let recovered_intakes = executor
        .recover_prompt_intakes_on_startup()
        .await
        .map_err(|error| DiscordRuntimeError::PromptIntakeRecovery(error.to_string()))?;
    if recovered_intakes != 0 {
        eprintln!("rust_prompt_intake_startup_recovered: {recovered_intakes}");
    }
    Ok((queue, executor))
}

fn reconcile_bridge_state(
    bridge_state: &BridgeState,
    mirror_db: &std::path::Path,
) -> Result<(), DiscordRuntimeError> {
    'tracked: for source in bridge_state.tracked_thread_ids()? {
        let mut current = source.clone();
        let mut visited = BTreeSet::from([current.clone()]);
        let mut transfers = Vec::new();
        loop {
            let next = match completed_app_server_fork_target_for_source(mirror_db, &current) {
                Ok(next) => next,
                Err(StoreError::DeadGenerationTargetHeld(_)) => continue 'tracked,
                Err(error) => return Err(error.into()),
            };
            let Some(target) = next else {
                break;
            };
            if !visited.insert(target.clone()) {
                return Err(DiscordRuntimeError::BridgeForkCycle { thread_id: source });
            }
            transfers.push((current, target.clone()));
            current = target;
        }
        for (source, target) in transfers {
            bridge_state.apply_thread_fork(&source, &target)?;
        }
    }
    Ok(())
}

pub(super) fn prepare_storage(paths: &RuntimePaths) -> Result<(), DiscordRuntimeError> {
    let _ = CodexThreadStore::open(&paths.state_db)?;
    let _ = open_initialized(&paths.mirror_db)?;
    Ok(())
}

pub(super) async fn load_autocomplete(
    server: &ResidentAppServer,
) -> Result<AutocompleteCatalog, AppServerError> {
    let model_list = server
        .execute(
            cdr_app_server::requests::list_models(),
            Some(server.generation()),
        )
        .await?;
    Ok(AutocompleteCatalog::from_model_list(&model_list))
}

pub(super) fn interaction_policy(
    config: &RuntimeConfig,
    mirror_db: &std::path::Path,
) -> Result<InteractionAccessPolicy, StoreError> {
    let mut policy = InteractionAccessPolicy {
        allowed_channel_ids: config.allowed_channel_ids.clone(),
        allowed_user_ids: config.allowed_user_ids.clone(),
        allow_all_channels: config.allow_all_channels,
        ..InteractionAccessPolicy::default()
    };
    refresh_mirror_policy(&mut policy, mirror_db)?;
    Ok(policy)
}

pub(super) fn refresh_mirror_policy(
    policy: &mut InteractionAccessPolicy,
    mirror_db: &std::path::Path,
) -> Result<(), StoreError> {
    let ids = remaining_discord_ids(mirror_db)?;
    let targets = mirror_targets(mirror_db, i64::MAX)?;
    policy.mirrored_channel_ids = ids
        .thread_ids
        .into_iter()
        .chain(ids.project_channel_ids)
        .chain(
            targets
                .into_iter()
                .flat_map(|target| [target.discord_channel_id, target.discord_thread_id]),
        )
        .filter_map(|id| u64::try_from(id).ok())
        .collect::<BTreeSet<_>>();
    Ok(())
}

fn archive_delete_paths(paths: &RuntimePaths) -> ArchiveDeletePaths {
    ArchiveDeletePaths {
        state_db: paths.state_db.clone(),
        log_db: paths.log_db.clone(),
        global_state: paths.global_state.clone(),
        bridge_state: paths.bridge_state.clone(),
        session_index: paths.session_index.clone(),
        archived_sessions: paths.archived_sessions.clone(),
        backup_root: paths.maintenance_backup_root.clone(),
    }
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
