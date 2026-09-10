//! Freeze lifecycle routing independently from settings mutation custody.
use crate::{
    message_plan::{MessagePlan, MessagePlanError},
    message_worker::MessageAdmissionError,
    settings_binding::{
        SettingsBinding, SettingsRoute, SettingsTargetResolver, is_request_rejection,
    },
};

pub(super) fn bind(
    plan: &mut Result<MessagePlan, MessagePlanError>,
    routed: Option<&str>,
    resolver: &SettingsTargetResolver,
    channel: u64,
) -> Result<Option<Box<SettingsBinding>>, MessageAdmissionError> {
    let Ok(MessagePlan::Execute(action)) = plan else {
        return Ok(None);
    };
    let binding = match resolver.bind_lifecycle(action, channel) {
        Ok(binding) => binding,
        Err(error) if is_request_rejection(&error) => {
            *plan = Ok(MessagePlan::Respond(format!("ERROR: {error}")));
            return Ok(None);
        }
        Err(error) => return Err(cdr_store::StoreError::Integrity(error.to_string()).into()),
    };
    if let Some(binding) = &binding {
        let same_route = match binding.route {
            SettingsRoute::Explicit => true,
            SettingsRoute::Mapped => routed == Some(binding.target.as_str()),
            SettingsRoute::Selected => routed.is_none(),
        };
        if !same_route {
            *plan = Ok(MessagePlan::Respond("ERROR: lifecycle mapping changed during admission; no lifecycle operation was sent".into()));
            return Ok(None);
        }
    }
    Ok(binding.map(Box::new))
}
