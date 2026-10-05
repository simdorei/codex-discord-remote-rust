//! Frozen admission and one-use durable custody reach the final effect boundary.
use super::{ActionError, ActionExecutor};
use crate::{
    command_plan::CommandAction,
    queue_runner::TurnBackend,
    settings_binding::{SettingsBinding, SettingsTargetResolver},
};
use cdr_app_server::AppServerError;
use cdr_store::ingress::{RecoveryClaim, StoredIngress};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub(super) struct RecoveryGuard {
    binding: SettingsBinding,
    binding_json: serde_json::Value,
    resolver: SettingsTargetResolver,
    database: PathBuf,
    channel: u64,
    claim: Option<RecoveryClaim>,
}

impl RecoveryGuard {
    pub(super) fn target(&self) -> &str {
        &self.binding.target
    }

    pub(super) fn check(&self) -> Result<(), ActionError> {
        self.check_in(&cdr_store::schema::open_initialized(&self.database)?)?;
        Ok(())
    }

    pub(super) fn check_in(&self, db: &rusqlite::Connection) -> cdr_store::Result<()> {
        self.resolver
            .validate_lifecycle(&self.binding, self.channel)
            .map_err(|e| cdr_store::StoreError::Integrity(e.to_string()))?;
        if self.binding.command.get("Repair").is_some()
            && db.query_row(
                "SELECT EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)",
                [self.target()],
                |row| row.get::<_, bool>(0),
            )?
        {
            return Err(cdr_store::StoreError::Integrity(
                "repair target has an archive fence; original archive intent is preserved; no tool reset was authorized".into(),
            ));
        }
        if let Some(claim) = &self.claim {
            claim.validate_in(db)
        } else {
            let channel = i64::try_from(self.channel).map_err(|_| {
                cdr_store::StoreError::Integrity("recovery channel out of range".into())
            })?;
            cdr_store::ingress::validate_recovery_binding_in(db, &self.binding_json, channel)
        }
    }

    pub(super) fn rpc_check(
        &self,
        rejected: Option<Arc<AtomicBool>>,
    ) -> Arc<dyn Fn() -> Result<(), AppServerError> + Send + Sync> {
        let guard = self.clone();
        Arc::new(move || {
            guard.check().map_err(|error| {
                if let Some(rejected) = &rejected {
                    rejected.store(true, Ordering::Release);
                }
                AppServerError::InvalidReply {
                    message: error.to_string(),
                }
            })
        })
    }
}

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn freeze_recovery(
        &self,
        channel: u64,
        action: &CommandAction,
    ) -> Result<RecoveryGuard, ActionError> {
        let binding = self
            .settings_resolver()
            .bind_lifecycle(action, channel)?
            .ok_or_else(|| ActionError::Invalid("recovery binding is missing".into()))?;
        self.recovery_guard(channel, binding)
    }

    pub(super) fn claim_recovery_guard(
        &self,
        channel: u64,
        binding: SettingsBinding,
        record: &StoredIngress,
    ) -> Result<RecoveryGuard, ActionError> {
        let mut guard = self.recovery_guard(channel, binding)?;
        guard.claim = Some(cdr_store::ingress::claim_recovery(&self.mirror_db, record)?);
        guard.check()?;
        Ok(guard)
    }

    fn recovery_guard(
        &self,
        channel: u64,
        binding: SettingsBinding,
    ) -> Result<RecoveryGuard, ActionError> {
        let binding_json =
            serde_json::to_value(&binding).map_err(|e| ActionError::Invalid(e.to_string()))?;
        let guard = RecoveryGuard {
            binding,
            binding_json,
            resolver: self.settings_resolver(),
            database: self.mirror_db.clone(),
            channel,
            claim: None,
        };
        guard.check()?;
        Ok(guard)
    }
}
