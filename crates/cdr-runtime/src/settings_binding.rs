//! Freeze settings routing before asynchronous command admission.
use crate::{action_executor::ActionError, bridge_state::BridgeState, command_plan::CommandAction};
use cdr_codex_state::{CodexThreadStore, resolve_thread_ref};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct SettingsTargetResolver {
    state_db: PathBuf,
    mirror_db: PathBuf,
    bridge: Arc<BridgeState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsBinding {
    pub target: String,
    pub route: SettingsRoute,
    pub command: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SettingsRoute {
    Explicit,
    Mapped,
    Selected,
}

impl SettingsTargetResolver {
    #[must_use]
    pub const fn new(state_db: PathBuf, mirror_db: PathBuf, bridge: Arc<BridgeState>) -> Self {
        Self {
            state_db,
            mirror_db,
            bridge,
        }
    }

    pub fn bind(
        &self,
        action: &CommandAction,
        channel: u64,
    ) -> Result<Option<SettingsBinding>, ActionError> {
        match action {
            CommandAction::Settings {
                reference,
                model,
                effort,
                speed,
            } => {
                if model.is_none() && effort.is_none() && speed.is_none() {
                    return Ok(None);
                }
                self.bind_reference(action, reference.as_deref(), channel, "settings")
                    .map(Some)
            }
            CommandAction::AutoReserve { reference, .. } => self
                .bind_reference(action, reference.as_deref(), channel, "settings")
                .map(Some),
            _ => Ok(None),
        }
    }

    pub(crate) fn bind_lifecycle(
        &self,
        action: &CommandAction,
        channel: u64,
    ) -> Result<Option<SettingsBinding>, ActionError> {
        let (CommandAction::Archive { reference } | CommandAction::Resume { reference }) = action
        else {
            return Ok(None);
        };
        self.bind_reference(action, reference.as_deref(), channel, "lifecycle")
            .map(Some)
    }

    fn bind_reference(
        &self,
        action: &CommandAction,
        reference: Option<&str>,
        channel: u64,
        label: &str,
    ) -> Result<SettingsBinding, ActionError> {
        let store = CodexThreadStore::open(&self.state_db)?;
        let selected = self.bridge.selected_thread_id()?;
        let (target, route) = if let Some(reference) = reference {
            let reference = reference.trim();
            let thread = if let Some(thread) = store.load_thread(reference, false)? {
                thread
            } else {
                resolve_thread_ref(
                    &store.load_recent_threads(0)?,
                    reference,
                    selected.as_deref(),
                    false,
                )?
                .clone()
            };
            (thread.id, SettingsRoute::Explicit)
        } else if let Some(target) = self.mapped(channel)? {
            (target, SettingsRoute::Mapped)
        } else {
            (
                selected.ok_or(ActionError::NoTarget)?,
                SettingsRoute::Selected,
            )
        };
        if store.load_thread(&target, false)?.is_none() {
            return Err(ActionError::Invalid(format!(
                "{label} admission target is not an active original thread"
            )));
        }
        let binding = SettingsBinding {
            target,
            route,
            command: serde_json::to_value(action)
                .map_err(|error| ActionError::Invalid(error.to_string()))?,
        };
        self.validate_route(&binding, channel, label)?;
        Ok(binding)
    }

    pub fn validate(&self, binding: &SettingsBinding, channel: u64) -> Result<(), ActionError> {
        self.validate_route(binding, channel, "settings")
    }

    pub(crate) fn validate_lifecycle(
        &self,
        binding: &SettingsBinding,
        channel: u64,
    ) -> Result<(), ActionError> {
        self.validate_route(binding, channel, "lifecycle")
    }

    fn validate_route(
        &self,
        binding: &SettingsBinding,
        channel: u64,
        label: &str,
    ) -> Result<(), ActionError> {
        let valid = match binding.route {
            SettingsRoute::Explicit => true,
            SettingsRoute::Mapped => {
                self.mapped(channel)?.as_deref() == Some(binding.target.as_str())
            }
            SettingsRoute::Selected => {
                self.mapped(channel)?.is_none()
                    && self.bridge.selected_thread_id()?.as_deref() == Some(binding.target.as_str())
            }
        };
        if valid {
            Ok(())
        } else {
            Err(ActionError::Invalid(format!(
                "{label} target changed after admission; no replacement target will be used"
            )))
        }
    }

    fn mapped(&self, channel: u64) -> Result<Option<String>, ActionError> {
        Ok(cdr_store::mapping::mirrored_thread_id(
            &self.mirror_db,
            Some(i64::try_from(channel).map_err(|_| ActionError::IntegerRange)?),
        )?)
    }
}

#[must_use]
pub fn is_settings_mutation(action: &CommandAction) -> bool {
    matches!(action,
        CommandAction::Settings {model,effort,speed,..} if model.is_some() || effort.is_some() || speed.is_some()
    ) || matches!(action, CommandAction::AutoReserve { .. })
}

/// Input/routing rejection is not evidence of database or custody corruption.
#[must_use]
pub fn is_request_rejection(error: &ActionError) -> bool {
    matches!(
        error,
        ActionError::NoTarget | ActionError::Resolve(_) | ActionError::Invalid(_)
    )
}
