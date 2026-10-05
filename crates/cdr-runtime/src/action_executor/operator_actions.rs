use cdr_app_server::AppServerError;
use cdr_app_server::requests::{interrupt_turn, steer_turn};

use super::{ActionContext, ActionError, ActionExecutor, ActionResult, immediate};
use crate::action_executor::app_server_requests::resume_request;
use crate::queue_runner::TurnBackend;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn open_thread(
        &self,
        reference: &str,
        abort: bool,
    ) -> Result<ActionResult, ActionError> {
        let thread = self.resolve_thread(0, Some(reference))?;
        let target = self.prepare_action_target(&thread.id, "reference").await?;
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        if let Some(turn_id) = server.active_turn_id(&target.thread_id).await? {
            if !abort {
                return Err(ActionError::Invalid(format!(
                    "thread {} has an active turn; use open_abort to interrupt it",
                    target.thread_id
                )));
            }
            server
                .execute(
                    interrupt_turn(&target.thread_id, &turn_id),
                    Some(server.generation()),
                )
                .await?;
        }
        server
            .execute(
                resume_request(&target.thread_id, self.app_server_resume_timeout),
                Some(server.generation()),
            )
            .await?;
        self.bridge_state
            .set_selected_thread_id(Some(&target.thread_id))?;
        Ok(immediate(format!(
            "Opened Codex thread\nthread_id: {}\ntitle: {}",
            target.thread_id, thread.title
        )))
    }

    pub(super) async fn stop_thread(
        &self,
        context: ActionContext,
        reference: Option<&str>,
    ) -> Result<ActionResult, ActionError> {
        let action = crate::command_plan::CommandAction::Stop {
            reference: reference.map(str::to_owned),
        };
        let binding = self
            .settings_resolver()
            .bind_lifecycle(&action, context.channel_id)?
            .ok_or_else(|| ActionError::Invalid("stop target was not bound".into()))?;
        self.stop_bound(context, &binding, None).await
    }

    pub(super) async fn approval(
        &self,
        channel_id: u64,
        user_id: u64,
    ) -> Result<ActionResult, ActionError> {
        let thread_id = self.target(channel_id)?.0;
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let prompts = crate::server_prompt_redisplay::prepare(
            &self.mirror_db,
            server,
            &thread_id,
            channel_id,
            user_id,
        )
        .await?;
        if prompts.is_empty() {
            return Ok(immediate(format!(
                "No pending Codex approval or input request for {thread_id}."
            )));
        }
        Ok(ActionResult {
            text: format!(
                "Existing Codex approval/input requests: {}\nthread: {thread_id}",
                prompts.len()
            ),
            waits_for_final: false,
            ui: Some(super::ActionUi::ServerPrompts { prompts }),
        })
    }

    pub(super) async fn steer(
        &self,
        channel_id: u64,
        prompt: &str,
    ) -> Result<ActionResult, ActionError> {
        let thread_id = self.canonicalize_completed_target(&self.target(channel_id)?.0)?;
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let _control = self.control_lock(&thread_id).await?;
        let (turn_id, generation) = self
            .verified_control_turn(channel_id, &thread_id, None)
            .await?;
        cdr_store::mirror::record_user_origin(
            &self.mirror_db,
            &thread_id,
            &turn_id,
            prompt,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs_f64(),
        )?;
        server
            .execute(steer_turn(&thread_id, prompt, &turn_id), Some(generation))
            .await?;
        Ok(immediate(format!(
            "Steering request submitted to {thread_id}."
        )))
    }
}

pub(super) fn original_owner_error(
    operation: &str,
    thread_id: &str,
    error: AppServerError,
) -> ActionError {
    if matches!(
        &error,
        AppServerError::Remote {
            code: -32_600,
            message,
            ..
        } if message.contains("already has an active writer")
    ) {
        return ActionError::Invalid(format!(
            "{operation} requires the app-server that owns original thread {thread_id}; no fork was used because that would change which thread is affected. app-server error: {error}"
        ));
    }
    ActionError::AppServer(error)
}
