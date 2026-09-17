//! Classify live settings before applying ordinary-quota admission shortcuts.
use super::{Identity, ReserveAutoController, account_id, held, ordinary_allowed, store_failure};
use crate::action_executor::{model_catalog::reserve, settings_action::snapshot::Settings};
use crate::queue_runner::BackendFailure;
use cdr_store::reserve_policy::{self, Policy};
use serde_json::Value;

type UsageClaim = reserve_policy::usage_fence::Claim;

impl ReserveAutoController {
    pub(super) async fn prepare_ordinary(
        &self,
        thread_id: &str,
        policy: &Policy,
        identity: Identity,
        rates: &Value,
        usage_failure: Option<UsageClaim>,
    ) -> Result<(), BackendFailure> {
        // A manual Reserve selection followed by auto/on still has an ordinary
        // policy. Only an exact idle observation identifies the actual model.
        let resumed = self.resume(thread_id, identity).await?;
        let current = Settings::from_resume(&resumed).map_err(|error| held(error.to_string()))?;
        self.require_policy_snapshot(policy)?;
        if current.model == reserve::MODEL {
            // General quota recovery is not evidence of Reserve capacity. Keep
            // the failure claim until exact Reserve settings/capacity validate.
            return self
                .verify_reserve_settings(thread_id, identity, rates, policy, usage_failure)
                .await;
        }
        match ordinary_allowed(rates) {
            Ok(true) => self.resolve_ordinary_usage(thread_id, usage_failure),
            Ok(false) => {
                let account = account_id(rates)?;
                self.enter_reserve(thread_id, policy, &account, identity, rates, usage_failure)
                    .await
            }
            Err(error) if usage_failure.is_none() => {
                // Preserve the optional-field contract only for an observed
                // ordinary model with no unresolved usage failure.
                eprintln!(
                    "reserve_auto_ordinary_quota_unknown thread_id={thread_id} reason={}",
                    error.message
                );
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn resolve_ordinary_usage(
        &self,
        thread_id: &str,
        claim: Option<UsageClaim>,
    ) -> Result<(), BackendFailure> {
        if let Some(claim) = claim
            && !reserve_policy::resolve_usage_failure_claim(
                &self.db_path,
                thread_id,
                claim,
                "ordinary quota recovery confirmed",
            )
            .map_err(|error| store_failure(&error))?
        {
            return Err(held("automatic usage-limit handling was superseded"));
        }
        Ok(())
    }
}
