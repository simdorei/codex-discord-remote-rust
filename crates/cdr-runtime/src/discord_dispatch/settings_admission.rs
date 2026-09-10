use crate::settings_binding::{
    SettingsBinding, SettingsTargetResolver, is_request_rejection, is_settings_mutation,
};
use cdr_discord::interaction::RoutedWork;
use cdr_store::StoreError;

#[derive(Default)]
pub(super) struct SettingsAdmission {
    pub binding: Option<SettingsBinding>,
    pub rejection: Option<String>,
}

pub(super) fn prepare(
    work: &RoutedWork,
    resolver: Option<&SettingsTargetResolver>,
    channel: u64,
) -> Result<SettingsAdmission, StoreError> {
    let RoutedWork::Slash(invocation) = work else {
        return Ok(SettingsAdmission::default());
    };
    if invocation.name != "settings" {
        return Ok(SettingsAdmission::default());
    }
    let action = match crate::command_plan::plan_slash(invocation) {
        Ok(action) => action,
        Err(error) => {
            return Ok(SettingsAdmission {
                binding: None,
                rejection: Some(error.to_string()),
            });
        }
    };
    if !is_settings_mutation(&action) {
        return Ok(SettingsAdmission::default());
    }
    let resolver = resolver.ok_or_else(|| {
        StoreError::Integrity("settings admission resolver is unavailable".into())
    })?;
    match resolver.bind(&action, channel) {
        Ok(binding) => Ok(SettingsAdmission {
            binding,
            rejection: None,
        }),
        Err(error) if is_request_rejection(&error) => Ok(SettingsAdmission {
            binding: None,
            rejection: Some(error.to_string()),
        }),
        Err(error) => Err(StoreError::Integrity(error.to_string())),
    }
}
