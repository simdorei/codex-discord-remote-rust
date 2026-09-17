//! Apply the default/high/medium policy to an already-Reserve idle thread.
use super::{Identity, ReserveAutoController, account_id, held, store_failure, wire_generation};
use crate::action_executor::{model_catalog::reserve, settings_action};
use crate::queue_runner::BackendFailure;
use cdr_app_server::requests::{ServiceTierUpdate, ThreadSettingsUpdate, list_models};
use cdr_store::reserve_policy::{self, Policy};
use serde_json::Value;
use settings_action::snapshot::Settings;

type UsageClaim = reserve_policy::usage_fence::Claim;

impl ReserveAutoController {
    pub(super) async fn verify_reserve_settings(
        &self,
        thread_id: &str,
        identity: Identity,
        rates: &Value,
        policy: &Policy,
        usage_failure: Option<UsageClaim>,
    ) -> Result<(), BackendFailure> {
        let account = account_id(rates)?;
        // An owned automatic episode keeps its restore contract. A manually
        // selected Reserve model has no ordinary quota precondition for reuse.
        let ordinary = (policy.state == "reserve").then_some(false);
        if policy.state == "reserve" {
            self.require_episode_account(thread_id, policy, &account)?;
        }
        let resumed = self.resume(thread_id, identity).await?;
        let current = Settings::from_resume(&resumed).map_err(|error| held(error.to_string()))?;
        self.require_existing_reserve_owner(policy, &current)?;
        self.require_policy_snapshot(policy)?;
        let models = self
            .server
            .execute(list_models(), Some(wire_generation(identity.1)?))
            .await
            .map_err(|error| held(error.to_string()))?;
        self.require_identity(identity).await?;
        let catalog = reserve::catalog(&models, rates).map_err(|error| held(error.to_string()))?;
        let effort = reserve::auto_effort(&catalog).map_err(|error| held(error.to_string()))?;
        let update = ThreadSettingsUpdate {
            model: Some(reserve::MODEL.into()),
            effort: Some(effort.clone()),
            effort_clear: false,
            service_tier: ServiceTierUpdate::Set("default".into()),
        };
        if current.matches(&update) {
            self.verify_final_reserve(thread_id, identity, &account, &update, ordinary)
                .await?;
            self.require_policy_snapshot(policy)?;
            return self.resolve_verified_usage(thread_id, usage_failure);
        }
        // This is a new, recorded settings change, not a validation-only fallback.
        self.require_account(identity, &account, ordinary).await?;
        let claim = reserve_policy::alignment::begin(
            &self.db_path,
            policy,
            reserve_policy::EpisodeIdentity {
                account_id: &account,
                process_id: identity.0,
                generation: identity.1,
            },
            &effort,
        )
        .map_err(|error| store_failure(&error))?
        .ok_or_else(|| held("Reserve effort alignment was superseded"))?;
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
            .map_err(|error| held(error.to_string()))?;
            self.verify_final_reserve(thread_id, identity, &account, &update, ordinary)
                .await?;
            if !reserve_policy::alignment::finish(&self.db_path, &claim)
                .map_err(|error| store_failure(&error))?
            {
                return Err(held(
                    "Reserve effort completion or usage fence was superseded",
                ));
            }
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            let _ = reserve_policy::mark_unknown_claim(
                &self.db_path,
                thread_id,
                claim.revision(),
                &error.message,
            );
        }
        result
    }

    fn require_existing_reserve_owner(
        &self,
        policy: &Policy,
        current: &Settings,
    ) -> Result<(), BackendFailure> {
        let invalid = current.model != reserve::MODEL
            || !current.effort_present
            || current.tier.as_deref() != Some("default")
            || (policy.state == "reserve"
                && (policy.applied_model.as_deref() != Some(current.model.as_str())
                    || policy.applied_effort != current.effort
                    || policy.applied_tier != current.tier));
        if invalid {
            if policy.state == "reserve" {
                self.mark_unknown(
                    policy,
                    &policy.thread_id,
                    "Reserve settings changed outside this episode",
                )?;
            }
            return Err(held("Reserve settings changed outside this episode"));
        }
        Ok(())
    }

    pub(super) fn require_policy_snapshot(&self, expected: &Policy) -> Result<(), BackendFailure> {
        if reserve_policy::get(&self.db_path, &expected.thread_id)
            .map_err(|error| store_failure(&error))?
            .as_ref()
            != Some(expected)
            || cdr_store::archive_fence::target_is_fenced(&self.db_path, &expected.thread_id)
                .map_err(|error| store_failure(&error))?
            || cdr_store::dead_generation::target_is_held(&self.db_path, &expected.thread_id)
                .map_err(|error| store_failure(&error))?
        {
            return Err(held("automatic Reserve policy was superseded or held"));
        }
        Ok(())
    }

    fn resolve_verified_usage(
        &self,
        thread_id: &str,
        claim: Option<UsageClaim>,
    ) -> Result<(), BackendFailure> {
        if let Some(claim) = claim
            && !reserve_policy::resolve_usage_failure_claim(
                &self.db_path,
                thread_id,
                claim,
                "verified Reserve capacity and exact effort accepted the usage failure",
            )
            .map_err(|error| store_failure(&error))?
        {
            return Err(held("automatic usage-limit handling was superseded"));
        }
        Ok(())
    }

    // Shared by entry, no-op reuse and realignment. Never select another effort here.
    pub(super) async fn verify_final_reserve(
        &self,
        thread_id: &str,
        identity: Identity,
        account: &str,
        update: &ThreadSettingsUpdate,
        ordinary: Option<bool>,
    ) -> Result<(), BackendFailure> {
        let final_rates = self.require_account(identity, account, ordinary).await?;
        let models = self
            .server
            .execute(list_models(), Some(wire_generation(identity.1)?))
            .await
            .map_err(|_| held("final Reserve model metadata is unavailable"))?;
        self.require_identity(identity).await?;
        let catalog = reserve::catalog(&models, &final_rates)
            .map_err(|_| held("final Reserve capacity/model validation failed"))?;
        let effort = update
            .effort
            .as_deref()
            .ok_or_else(|| held("exact Reserve effort is missing"))?;
        reserve::validate_auto_effort(&catalog, effort)
            .map_err(|_| held("final exact Reserve effort validation failed"))?;
        // Confirm the live values again after the account/model awaits, not only the ACK.
        let resumed = self.resume(thread_id, identity).await?;
        let observed = Settings::from_resume(&resumed)
            .map_err(|_| held("final Reserve settings observation is invalid"))?;
        if !observed.matches(update) || !observed.effort_present {
            return Err(held(
                "final Reserve settings do not match the applied values",
            ));
        }
        self.require_identity(identity).await
    }
}
