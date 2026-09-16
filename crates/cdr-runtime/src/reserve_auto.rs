//! Automatic ordinary-quota/Reserve switching.
//
// This module deliberately changes settings only after a fresh account read and
// never starts the failed prompt again. Queue admission and the target lock are
// owned by QueueCoordinator for recovery; normal turns enter this controller
// while already holding that same lock.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cdr_app_server::requests::{
    ServiceTierUpdate, ThreadSettingsUpdate, list_models, rate_limits, resume_thread_with_timeout,
};
use cdr_app_server::{ResidentAppServer, extract_thread_id};
use cdr_store::reserve_policy::{self, Policy};
use serde_json::Value;

use crate::action_executor::{ActionError, model_catalog::reserve, settings_action};
use crate::queue_runner::BackendFailure;

type Identity = (Option<i64>, i64);

#[derive(Clone)]
pub struct ReserveAutoController {
    server: Arc<ResidentAppServer>,
    db_path: PathBuf,
}

impl ReserveAutoController {
    pub fn new(server: Arc<ResidentAppServer>, db_path: impl Into<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            server,
            db_path: db_path.into(),
        })
    }

    pub fn poll_recovery_after(&self, after: Option<&str>) -> Vec<String> {
        match reserve_policy::recovery_candidates_after(&self.db_path, after) {
            Ok(candidates) => candidates,
            Err(error) => {
                eprintln!("reserve_auto_recovery_candidates_failed error={error}");
                Vec::new()
            }
        }
    }

    pub async fn recover_target(&self, thread_id: &str) -> Result<(), BackendFailure> {
        // A candidate list is not permission: recheck after queue acquires the lock.
        if cdr_store::archive_fence::target_is_fenced(&self.db_path, thread_id)
            .map_err(|error| store_failure(&error))?
        {
            return Ok(());
        }
        let policy =
            reserve_policy::get(&self.db_path, thread_id).map_err(|error| store_failure(&error))?;
        if policy
            .as_ref()
            .is_none_or(|p| p.state != "reserve" || !matches!(p.mode.as_str(), "auto" | "on"))
        {
            return Ok(());
        }
        match self.server.active_turn_id(thread_id).await {
            Ok(Some(_)) => Err(held("automatic recovery found an active turn")),
            Err(error) => Err(action_failure(&ActionError::from(error))),
            Ok(None) => self.prepare_turn(thread_id).await,
        }
    }

    pub fn set_manual_mode(&self, thread_id: &str, enabled: bool) -> Result<String, ActionError> {
        reserve_policy::set_mode(&self.db_path, thread_id, if enabled { "on" } else { "off" })?;
        Ok(format!(
            "자동 Reserve 전환을 {}했습니다. 실패한 요청은 재실행하지 않습니다.",
            if enabled { "활성화" } else { "비활성화" }
        ))
    }

    pub fn manual_override(&self, thread_id: &str) -> Result<(), ActionError> {
        reserve_policy::set_mode(&self.db_path, thread_id, "manual")?;
        Ok(())
    }

    pub async fn prepare_turn(&self, thread_id: &str) -> Result<(), BackendFailure> {
        if cdr_store::archive_fence::target_is_fenced(&self.db_path, thread_id)
            .map_err(|error| store_failure(&error))?
        {
            return Err(held("archive lifecycle fence blocks automatic settings"));
        }
        let policy = reserve_policy::ensure(&self.db_path, thread_id)
            .map_err(|error| store_failure(&error))?;
        let usage_failure_pending =
            reserve_policy::usage_failure_unresolved(&self.db_path, thread_id)
                .map_err(|error| store_failure(&error))?;
        let usage_failure = if usage_failure_pending {
            Some(
                reserve_policy::usage_failure_claim(&self.db_path, thread_id)
                    .map_err(|error| store_failure(&error))?
                    .ok_or_else(|| held("automatic usage-limit handling claim disappeared"))?,
            )
        } else {
            None
        };
        if matches!(policy.mode.as_str(), "off" | "manual") {
            if usage_failure_pending {
                return Err(held(
                    "disabled automatic policy still has an unresolved usage failure",
                ));
            }
            if matches!(
                policy.state.as_str(),
                "entering" | "restoring" | "unknown" | "held"
            ) {
                return Err(held(
                    "disabled automatic policy still has an unresolved settings outcome",
                ));
            }
            return Ok(());
        }
        let identity = self.identity().await;
        let rates = self.fresh_rates(identity).await?;
        let result = match policy.state.as_str() {
            "ordinary" => {
                self.prepare_ordinary(thread_id, &policy, identity, &rates, usage_failure)
                    .await
            }
            "reserve" => {
                if ordinary_allowed(&rates)? {
                    self.resolve_ordinary_usage(thread_id, usage_failure)?;
                    let account = account_id(&rates)?;
                    self.restore(thread_id, &policy, &account, identity).await
                } else {
                    // Owned episodes retain their ordinary-recovery/restore
                    // contract. Actual Reserve under an ordinary policy is
                    // classified separately in prepare_ordinary.
                    self.verify_reserve_settings(
                        thread_id,
                        identity,
                        &rates,
                        &policy,
                        usage_failure,
                    )
                    .await
                }
            }
            "held" | "unknown" | "entering" | "restoring" => Err(held(
                "automatic Reserve state requires a fresh, unambiguous observation",
            )),
            _ => Err(held("invalid automatic Reserve state")),
        };
        if result.is_ok()
            && reserve_policy::usage_failure_unresolved(&self.db_path, thread_id)
                .map_err(|error| store_failure(&error))?
        {
            return Err(held(
                "automatic preparation completed while usage-limit handling remains unresolved",
            ));
        }
        result
    }

    pub async fn note_usage_limit(&self, thread_id: &str) {
        let policy = match reserve_policy::ensure(&self.db_path, thread_id) {
            Ok(policy) => policy,
            Err(error) => {
                eprintln!("reserve_auto_usage_policy_failed thread_id={thread_id} error={error}");
                return;
            }
        };
        if matches!(policy.mode.as_str(), "off" | "manual") {
            return;
        }
        if let Err(error) = reserve_policy::stage_usage_failure(
            &self.db_path,
            thread_id,
            "typed usage-limit failure requires automatic handling",
        ) {
            eprintln!("reserve_auto_usage_fence_stage_failed thread_id={thread_id} error={error}");
            return;
        }
        // Past failures are triggers, not authority. Reuse the same CURRENT
        // policy/account/quota/exact-settings checks as a new request. In
        // particular, unknown/in-flight settings are never replayed on restart.
        match cdr_store::archive_fence::target_is_fenced(&self.db_path, thread_id) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "reserve_auto_usage_archive_check_failed thread_id={thread_id} error={error}"
                );
                return;
            }
        }
        if let Err(error) = self.prepare_turn(thread_id).await {
            let _ = reserve_policy::mark_unknown_claim(
                &self.db_path,
                thread_id,
                policy.revision,
                &error.message,
            );
            eprintln!("reserve_auto_usage_transition_failed thread_id={thread_id} error={error}");
        }
    }

    async fn enter_reserve(
        &self,
        thread_id: &str,
        policy: &Policy,
        account: &str,
        identity: Identity,
        rates: &Value,
        usage_failure: Option<reserve_policy::usage_fence::Claim>,
    ) -> Result<(), BackendFailure> {
        let resumed = self.resume(thread_id, identity).await?;
        let current = settings_action::snapshot::Settings::from_resume(&resumed)
            .map_err(|error| action_failure(&error))?;
        if current.model == reserve::MODEL {
            return self
                .verify_reserve_settings(thread_id, identity, rates, policy, usage_failure)
                .await;
        }
        self.require_identity(identity).await?;
        let catalog = self
            .server
            .execute(list_models(), Some(wire_generation(identity.1)?))
            .await
            .map_err(|error| action_failure(&ActionError::from(error)))?;
        self.require_identity(identity).await?;
        let catalog = reserve::catalog(&catalog, rates).map_err(|error| action_failure(&error))?;
        let effort = reserve::auto_effort(&catalog).map_err(|error| held(error.to_string()))?;
        self.require_account(identity, account, Some(false)).await?;
        let Some(claim) = reserve_policy::begin_episode_claim(
            &self.db_path,
            thread_id,
            policy.revision,
            reserve_policy::EpisodeIdentity {
                account_id: account,
                process_id: identity.0,
                generation: identity.1,
            },
            (
                &current.model,
                current.effort.as_deref(),
                current.tier.as_deref(),
            ),
            current.effort_present,
            (Some(reserve::MODEL), Some(&effort), Some("default")),
        )
        .map_err(|error| store_failure(&error))?
        else {
            return Err(held("automatic Reserve transition was superseded"));
        };
        let update = ThreadSettingsUpdate {
            model: Some(reserve::MODEL.into()),
            effort: Some(effort),
            effort_clear: false,
            service_tier: ServiceTierUpdate::Set("default".into()),
        };
        let result = async {
            self.require_identity(identity).await?;
            settings_action::verification::apply_or_confirm(
                &self.server,
                thread_id,
                wire_generation(identity.1)?,
                current,
                &update,
            )
            .await
            .map_err(|error| action_failure(&error))?;
            self.require_identity(identity).await?;
            self.verify_final_reserve(thread_id, identity, account, &update, Some(false))
                .await?;
            if !reserve_policy::finish_episode_claim(
                &self.db_path,
                thread_id,
                claim,
                "entering",
                "reserve",
            )
            .map_err(|error| store_failure(&error))?
            {
                return Err(held("automatic Reserve completion was superseded"));
            }
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            let _ = reserve_policy::mark_unknown_claim(
                &self.db_path,
                thread_id,
                claim.revision,
                &error.message,
            );
        }
        result
    }

    async fn restore(
        &self,
        thread_id: &str,
        policy: &Policy,
        account: &str,
        identity: Identity,
    ) -> Result<(), BackendFailure> {
        self.require_episode_account(thread_id, policy, account)?;
        let Some(model) = &policy.previous_model else {
            self.mark_unknown(policy, thread_id, "previous model missing")?;
            return Err(held(
                "previous settings are incomplete; automatic restore was not guessed",
            ));
        };
        if !policy.previous_effort_present {
            self.mark_unknown(policy, thread_id, "previous effort field was not present")?;
            return Err(held(
                "previous settings are incomplete; automatic restore was not guessed",
            ));
        }
        let resumed = self.resume(thread_id, identity).await?;
        let current = settings_action::snapshot::Settings::from_resume(&resumed)
            .map_err(|error| action_failure(&error))?;
        if policy.applied_model.as_deref() != Some(reserve::MODEL)
            || current.model != policy.applied_model.as_deref().unwrap_or_default()
            || current.effort.as_deref() != policy.applied_effort.as_deref()
            || current.tier.as_deref() != policy.applied_tier.as_deref()
        {
            self.mark_unknown(
                policy,
                thread_id,
                "Reserve settings changed outside this episode",
            )?;
            return Err(held(
                "Reserve settings changed outside this episode; automatic restore stopped",
            ));
        }
        let update = ThreadSettingsUpdate {
            model: Some(model.clone()),
            effort: policy.previous_effort.clone(),
            effort_clear: policy.previous_effort.is_none(),
            service_tier: match policy.previous_tier.as_deref() {
                Some(tier) => ServiceTierUpdate::Set(tier.into()),
                None => ServiceTierUpdate::Clear,
            },
        };
        self.require_account(identity, account, Some(true)).await?;
        let Some(claim) = reserve_policy::begin_restore_claim(
            &self.db_path,
            thread_id,
            policy.revision,
            account,
            identity.0,
            identity.1,
        )
        .map_err(|error| store_failure(&error))?
        else {
            return Err(held("automatic restore was superseded"));
        };
        let result = async {
            self.require_identity(identity).await?;
            settings_action::verification::apply_or_confirm(
                &self.server,
                thread_id,
                wire_generation(identity.1)?,
                current,
                &update,
            )
            .await
            .map_err(|error| action_failure(&error))?;
            self.require_identity(identity).await?;
            let final_rates = self
                .fresh_rates(identity)
                .await
                .map_err(|error| held(error.message))?;
            if account_id(&final_rates)? != account || !ordinary_allowed(&final_rates)? {
                return Err(held("ordinary usage recovery was not confirmed"));
            }
            if !reserve_policy::finish_episode_claim(
                &self.db_path,
                thread_id,
                claim,
                "restoring",
                "ordinary",
            )
            .map_err(|error| store_failure(&error))?
            {
                return Err(held("automatic restore completion was superseded"));
            }
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            let _ = reserve_policy::mark_unknown_claim(
                &self.db_path,
                thread_id,
                claim.revision,
                &error.message,
            );
        }
        result
    }

    fn require_episode_account(
        &self,
        thread_id: &str,
        policy: &Policy,
        account: &str,
    ) -> Result<(), BackendFailure> {
        // A confirmed episode belongs to the conversation/account, not the
        // lifespan of the old resident. Its process/generation remain historical
        // evidence. Callers still freshly verify exact live settings and pin the
        // current resident with require_identity() around every settings RPC.
        if policy.account_id.as_deref() != Some(account) {
            self.mark_unknown(policy, thread_id, "account identity changed")?;
            return Err(held("account identity changed; automatic episode is held"));
        }
        Ok(())
    }

    async fn require_account(
        &self,
        identity: Identity,
        account: &str,
        ordinary: Option<bool>,
    ) -> Result<Value, BackendFailure> {
        let rates = self.fresh_rates(identity).await?;
        // None is only used to verify an already-observed Reserve model under
        // an ordinary policy, which has no automatic restore episode. Entry
        // still requires Some(false); restoring still requires Some(true).
        let permission_changed = match ordinary {
            Some(expected) => ordinary_allowed(&rates)? != expected,
            None => false,
        };
        if account_id(&rates)? != account || permission_changed {
            return Err(held(
                "account or quota permission changed during automatic settings operation",
            ));
        }
        Ok(rates)
    }

    fn mark_unknown(
        &self,
        policy: &Policy,
        thread_id: &str,
        reason: &str,
    ) -> Result<(), BackendFailure> {
        let changed =
            reserve_policy::mark_unknown_claim(&self.db_path, thread_id, policy.revision, reason)
                .map_err(|error| store_failure(&error))?;
        if !changed {
            return Err(held("automatic Reserve policy was superseded"));
        }
        Ok(())
    }

    async fn fresh_rates(&self, expected: Identity) -> Result<Value, BackendFailure> {
        self.require_identity(expected).await?;
        let value = self
            .server
            .execute(rate_limits(), Some(wire_generation(expected.1)?))
            .await
            .map_err(|error| {
                if cdr_app_server::is_usage_limit_error(error_data(&error)) {
                    BackendFailure::usage_limit(error.to_string())
                } else {
                    action_failure(&ActionError::from(error))
                }
            })?;
        self.require_identity(expected).await?;
        Ok(value)
    }

    async fn resume(&self, thread_id: &str, expected: Identity) -> Result<Value, BackendFailure> {
        self.require_identity(expected).await?;
        let result = self
            .server
            .execute(
                resume_thread_with_timeout(thread_id, Duration::from_secs(8)),
                Some(wire_generation(expected.1)?),
            )
            .await
            .map_err(|error| action_failure(&ActionError::from(error)))?;
        if extract_thread_id(&result).as_deref() != Some(thread_id) {
            return Err(held(
                "automatic resume returned a different or missing thread",
            ));
        }
        if result["thread"]["status"]["type"].as_str() != Some("idle") {
            return Err(held(
                "automatic settings require an exact idle thread observation",
            ));
        }
        self.require_identity(expected).await?;
        Ok(result)
    }

    async fn identity(&self) -> Identity {
        let snapshot = self.server.lifecycle_snapshot().await;
        (
            snapshot.process_id.map(i64::from),
            i64::try_from(snapshot.generation).unwrap_or(-1),
        )
    }

    async fn require_identity(&self, expected: Identity) -> Result<(), BackendFailure> {
        let snapshot = self.server.lifecycle_snapshot().await;
        let actual = (
            snapshot.process_id.map(i64::from),
            i64::try_from(snapshot.generation).unwrap_or(-1),
        );
        if expected.0.is_none()
            || expected.1 < 0
            || actual != expected
            || !snapshot.healthy
            || snapshot.quarantined
            || snapshot.restart_pending
        {
            return Err(held(
                "app-server resident identity changed or is unavailable",
            ));
        }
        Ok(())
    }
}

fn wire_generation(value: i64) -> Result<u64, BackendFailure> {
    u64::try_from(value).map_err(|_| held("app-server generation is outside the wire contract"))
}

fn ordinary_allowed(value: &Value) -> Result<bool, BackendFailure> {
    value.get("ordinaryUsageAllowed").map_or_else(
        || Err(held("ordinary usage availability is unknown")),
        |value| {
            value
                .as_bool()
                .ok_or_else(|| held("ordinary usage availability is unknown"))
        },
    )
}

fn account_id(value: &Value) -> Result<String, BackendFailure> {
    value
        .get("accountId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| held("account identity is unavailable"))
}

fn error_data(error: &cdr_app_server::AppServerError) -> Option<&Value> {
    match error {
        cdr_app_server::AppServerError::Remote { data, .. } => data.as_ref(),
        _ => None,
    }
}

fn action_failure(error: &ActionError) -> BackendFailure {
    BackendFailure::definite(error.to_string())
}

fn store_failure(error: &cdr_store::StoreError) -> BackendFailure {
    BackendFailure::definite(format!("automatic Reserve policy store failed: {error}"))
}

fn held(message: impl Into<String>) -> BackendFailure {
    BackendFailure::auto_reserve_held(message)
}

#[cfg(test)]
mod native_tests;

mod alignment;
mod preparation;
mod recovery;
pub(crate) use recovery::run_recovery_cycle;
