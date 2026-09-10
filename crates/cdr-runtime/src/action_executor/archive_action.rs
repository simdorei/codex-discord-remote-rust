use super::{ActionError, ActionExecutor, ActionResult, immediate};
use super::{app_server_requests::resume_request, operator_actions::original_owner_error};
use crate::queue_runner::TurnBackend;
use cdr_app_server::requests::{archive_thread, read_thread};
use std::collections::BTreeSet;
use std::time::Duration;
#[path = "archive_dispatch.rs"]
mod dispatch;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn archive_thread(
        &self,
        context: super::ActionContext,
        reference: Option<&str>,
    ) -> Result<ActionResult, ActionError> {
        self.archive_bound(context, reference, None).await
    }

    pub(super) async fn archive_bound(
        &self,
        context: super::ActionContext,
        reference: Option<&str>,
        binding: Option<&crate::settings_binding::SettingsBinding>,
    ) -> Result<ActionResult, ActionError> {
        let mut archive_sent = false;
        tokio::time::timeout(
            self.app_server_resume_timeout,
            self.archive_with_verification(context, reference, &mut archive_sent, binding),
        )
        .await
        .map_err(|_| {
            let phase = if archive_sent {
                "archive dispatch was attempted; delivery and outcome is unverified and some conversations may already be archived; do not automatically retry"
            } else {
                "no archive was sent"
            };
            ActionError::Invalid(format!("archive operation timed out; {phase}"))
        })?
    }

    async fn archive_with_verification(
        &self,
        context: super::ActionContext,
        reference: Option<&str>,
        archive_sent: &mut bool,
        binding: Option<&crate::settings_binding::SettingsBinding>,
    ) -> Result<ActionResult, ActionError> {
        let channel = context.channel_id;
        let own = self.archive_own_request(context, reference)?;
        let thread =
            self.resolve_thread(channel, binding.map(|b| b.target.as_str()).or(reference))?;
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let _guard = self.control_lock(&thread.id).await?;
        let generation = server.generation();
        self.validate_lifecycle_binding(binding, channel)?;
        self.archive_preflight(channel, reference, &thread.id, generation, own.as_deref())
            .await?;
        self.load_archive_target(&thread.id, generation).await?;
        let children = super::archive_scope::descendants(server, &thread.id, generation).await?;
        let mut child_guards = Vec::new();
        for child in &children {
            child_guards.push(self.control_lock(child).await?);
            self.archive_preflight(0, Some(child), child, generation, own.as_deref())
                .await?;
            self.load_archive_target(child, generation).await?;
        }
        if super::archive_scope::descendants(server, &thread.id, generation).await? != children {
            return Err(ActionError::Invalid(
                "archive descendant scope changed; no archive was sent".into(),
            ));
        }
        for child in &children {
            self.archive_preflight(0, Some(child), child, generation, own.as_deref())
                .await?;
        }
        self.archive_preflight(channel, reference, &thread.id, generation, own.as_deref())
            .await?;
        self.validate_lifecycle_binding(binding, channel)?;
        let mut scope = children;
        scope.insert(thread.id.clone());
        // This transaction commits before awaiting the app-server writer. Late
        // ingress is saved held; earlier ingress prevents this reservation.
        let reservation =
            cdr_store::archive_fence::reserve(&self.mirror_db, &scope, own.as_deref())?;
        *archive_sent = true;
        if let Err(error) = server
            .execute(archive_thread(&thread.id), Some(generation))
            .await
        {
            return Err(dispatch::failure(
                &self.mirror_db,
                &reservation,
                &thread.id,
                error,
            ));
        }
        self.verify_archived(&scope).await?;
        cdr_store::archive_fence::verified(&self.mirror_db, &reservation)?;
        if self
            .bridge_state
            .selected_thread_id()?
            .as_ref()
            .is_some_and(|selected| scope.contains(selected))
        {
            self.bridge_state.set_selected_thread_id(None)?;
        }
        Ok(immediate(format!(
            "Archived Codex thread {} ({} conversations, persisted state verified).",
            thread.id,
            scope.len()
        )))
    }

    async fn load_archive_target(&self, thread: &str, generation: u64) -> Result<(), ActionError> {
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let resumed = server
            .execute(
                resume_request(thread, self.app_server_resume_timeout),
                Some(generation),
            )
            .await
            .map_err(|error| original_owner_error("archive", thread, error))?;
        if resumed
            .pointer("/thread/id")
            .and_then(serde_json::Value::as_str)
            != Some(thread)
        {
            return Err(ActionError::Invalid(
                "archive resume returned a missing or different thread identity".into(),
            ));
        }
        let read = server
            .execute(read_thread(thread, false), Some(generation))
            .await?;
        if read
            .pointer("/thread/id")
            .and_then(serde_json::Value::as_str)
            != Some(thread)
            || read
                .pointer("/thread/status/type")
                .and_then(serde_json::Value::as_str)
                != Some("idle")
        {
            return Err(ActionError::Invalid(format!(
                "archive requires confirmed idle status for {thread}; no archive was sent"
            )));
        }
        Ok(())
    }

    async fn archive_preflight(
        &self,
        channel: u64,
        reference: Option<&str>,
        thread: &str,
        generation: u64,
        own: Option<&str>,
    ) -> Result<(), ActionError> {
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let lifecycle = server.lifecycle_snapshot().await;
        if !lifecycle.healthy
            || lifecycle.quarantined
            || lifecycle.restart_pending
            || server.generation() != generation
        {
            return Err(ActionError::Invalid(
                "archive connection is unknown or changed; no archive was sent".into(),
            ));
        }
        if reference.is_none() && self.target(channel)?.0 != thread {
            return Err(ActionError::Invalid(
                "archive room target changed; no archive was sent".into(),
            ));
        }
        if server.active_turn_id(thread).await?.is_some() {
            return Err(ActionError::Invalid(
                "refusing to archive a thread with an active turn".into(),
            ));
        }
        if let Some(request) =
            cdr_store::ingress::unfinished_for_archive(&self.mirror_db, thread, own)?
        {
            return Err(ActionError::Invalid(format!(
                "refusing to archive while request {request} has unfinished ingress; request is preserved, no archive was sent"
            )));
        }
        if cdr_store::queue::list(&self.mirror_db)?
            .iter()
            .any(|job| job.target_thread_id == thread)
            || cdr_store::prompt_intake::list_prompt_intakes(&self.mirror_db)?
                .iter()
                .any(|job| job.target_thread_id == thread)
        {
            return Err(ActionError::Invalid("refusing to archive a thread with queued, running, or intake work; requests are preserved".into()));
        }
        if cdr_codex_state::CodexThreadStore::open(&self.state_db)?
            .load_thread(thread, false)?
            .is_none()
        {
            return Err(ActionError::Invalid(
                "archive target is no longer an active stored thread".into(),
            ));
        }
        Ok(())
    }

    async fn verify_archived(&self, scope: &BTreeSet<String>) -> Result<(), ActionError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let store = cdr_codex_state::CodexThreadStore::open(&self.state_db)?;
            let mut unverified = Vec::new();
            for thread in scope {
                if store.load_thread(thread, true)?.is_none() {
                    unverified.push(thread.as_str());
                }
            }
            if unverified.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ActionError::Invalid(format!(
                    "archive was acknowledged but persisted state was not verified for [{}]; other scope members may already be archived; do not automatically retry",
                    unverified.join(", ")
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
