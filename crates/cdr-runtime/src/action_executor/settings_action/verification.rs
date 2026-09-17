use super::{ActionError, ready, snapshot::Settings};
use cdr_app_server::{
    ResidentAppServer, ResidentNotificationEvent, requests::ThreadSettingsUpdate,
};
use std::time::Duration;
use tokio::sync::broadcast;

pub(crate) async fn apply_or_confirm(
    server: &ResidentAppServer,
    thread: &str,
    generation: u64,
    current: Settings,
    update: &ThreadSettingsUpdate,
) -> Result<(Settings, bool), ActionError> {
    // The caller supplies this invocation's complete, exact-thread resume
    // response under its control lock, never cached/local settings. It checks
    // generation and route before this call and again before recording success.
    if current.matches(update) {
        return Ok((current, true));
    }
    // A real change still needs a post-dispatch observation; neither its ACK
    // nor a pre-dispatch match is sufficient. Do not add a retry on timeout.
    let notifications = server.subscribe_notifications();
    let before = server
        .update_settings_with_watermark(thread, update, generation)
        .await?;
    let applied = wait(server, thread, generation, before, update, notifications).await?;
    Ok((applied, false))
}

async fn wait(
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
