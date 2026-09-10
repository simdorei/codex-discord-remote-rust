use super::{ActionError, ready, snapshot::Settings};
use cdr_app_server::{
    ResidentAppServer, ResidentNotificationEvent, requests::ThreadSettingsUpdate,
};
use std::time::Duration;
use tokio::sync::broadcast;

pub(super) async fn wait(
    server: &ResidentAppServer,
    thread: &str,
    generation: u64,
    before: u64,
    update: &ThreadSettingsUpdate,
    mut events: broadcast::Receiver<ResidentNotificationEvent>,
) -> Result<Settings, ActionError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        ready(server, generation).await?;
        if let Some((revision, value)) = server.observed_thread_settings(thread, generation)?
            && revision > before
        {
            let observed = Settings::parse(&value)?;
            if !observed.matches(update) {
                return Err(ActionError::Invalid("settings update acknowledgement received, but observed values do not match; no success or local settings change recorded".into()));
            }
            return Ok(observed);
        }
        let event=tokio::time::timeout_at(deadline,events.recv()).await
            .map_err(|_|ActionError::Invalid("settings update acknowledgement received, but no fresh matching settings observation arrived; outcome unverified, do not blindly retry".into()))?
            .map_err(|_|ActionError::Invalid("settings observation stream was interrupted or lost events; outcome unverified".into()))?;
        match event {
            ResidentNotificationEvent::Gap { .. } => {
                return Err(ActionError::Invalid(
                    "settings observation gap; outcome unverified".into(),
                ));
            }
            ResidentNotificationEvent::Notification {
                generation: actual, ..
            } if actual != generation => {
                return Err(ActionError::Invalid(
                    "settings observation generation changed; outcome unverified".into(),
                ));
            }
            ResidentNotificationEvent::Notification { .. } => {}
        }
    }
}
