use cdr_app_server::{AppServerError, ResidentAppServer};
use cdr_discord::components::{BusyAction, ComponentId};
use cdr_discord::http::DiscordHttpError;
use cdr_store::StoreError;
use cdr_store::claims::{BusyChoice, claim_busy_choice};
use thiserror::Error;

use crate::action_executor::{ActionError, ActionExecutor};
use crate::discord_dispatch::InboundInteractionWork;
use crate::queue_runner::TurnBackend;

use super::confirmation::{
    ConfirmationError, ConfirmationPlan, busy_confirmation_plan, busy_ready_marker,
    confirmation_ready, record_confirmation_ready, recovery_error,
};
use super::failure::{ClaimFailureDisposition, retain_or_release_busy_claim};

mod control;
mod state;
use control::execute_busy_action;

pub use state::{BusyChoiceState, read_busy_choice_state};

#[derive(Debug, Error)]
pub enum BusyComponentError {
    #[error("this busy-choice button is no longer active")]
    Missing,
    #[error("busy interaction has no matching pre-ack authorization snapshot")]
    MissingAuthorizationSnapshot,
    #[error("only the original sender can choose this busy action")]
    WrongUser,
    #[error("the busy-choice button belongs to a different Discord channel")]
    WrongChannel,
    #[error("steering is not allowed for this busy choice; queue it instead")]
    SteerNotAllowed,
    #[error("this busy choice was already handled")]
    AlreadyHandled,
    #[error(
        "this busy action is claimed, but no durable success marker exists; \
         it may still be in flight or its result is indeterminate"
    )]
    ActionUnconfirmed,
    #[error(
        "busy action acceptance is indeterminate; claim retained to prevent duplicate execution: \
         {0}"
    )]
    ActionOutcomeIndeterminate(String),
    #[error("control was not dispatched: {0}")]
    ControlNotDispatched(String),
    #[error("the target Codex thread has no active turn to control")]
    NoActiveTurn,
    #[error("busy-choice record has no Codex thread target")]
    NoTarget,
    #[error("Discord identifier does not fit the SQLite integer contract")]
    IntegerRange,
    #[error(transparent)]
    Action(#[from] ActionError),
    #[error(transparent)]
    AppServer(#[from] AppServerError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Discord(#[from] DiscordHttpError),
    #[error(transparent)]
    Confirmation(#[from] ConfirmationError),
}

pub async fn handle_busy_component<B: TurnBackend>(
    work: &InboundInteractionWork,
    component: &ComponentId,
    executor: &ActionExecutor<B>,
    server: &ResidentAppServer,
) -> Result<ConfirmationPlan, BusyComponentError> {
    let ComponentId::Busy { choice_id, action } = component else {
        return Err(BusyComponentError::Missing);
    };
    let choice = authorized_choice(work, choice_id)?;
    validate_busy_choice(choice, *action, work.user_id.get(), work.channel_id.get())?;
    let now = super::now()?;
    let database = executor.mirror_db();
    let plan = busy_confirmation_plan(choice_id);
    let ready_marker = busy_ready_marker(choice_id, work.user_id.get(), work.channel_id.get());
    if confirmation_ready(database, &ready_marker, now)? {
        return Ok(plan);
    }
    if *action == BusyAction::Queue {
        // Queue acceptance and the confirmation receipt commit together. Do not
        // preclaim the button: a crash must always leave a recoverable prompt.
        executor.enqueue_busy_choice(choice).await?;
        return Ok(plan);
    }
    let state = read_busy_choice_state(database, choice_id, now)?;
    if state.as_ref().is_some_and(|state| state.claimed) {
        return confirmation_if_ready(database, &ready_marker, now, plan);
    }
    if state.is_none() {
        return Err(BusyComponentError::Missing);
    }
    if !claim_busy_choice(executor.mirror_db(), choice_id, now)? {
        return confirmation_if_ready(database, &ready_marker, now, plan);
    }
    let result = execute_busy_action(choice, *action, server, database, executor).await;
    match result {
        Ok(()) => {}
        Err(error) => {
            if retain_or_release_busy_claim(database, choice_id, &error)?
                == ClaimFailureDisposition::RetainIndeterminate
            {
                return Err(BusyComponentError::ActionOutcomeIndeterminate(
                    error.to_string(),
                ));
            }
            return Err(error);
        }
    }
    let completed_at = super::now().map_err(|error| recovery_error(&error))?;
    let _ = record_confirmation_ready(
        database,
        &ready_marker,
        completed_at,
        super::COMPONENT_CLAIM_TTL_SECONDS,
    )
    .map_err(|error| recovery_error(&error))?;
    Ok(plan)
}

pub(super) fn prepare_confirmation_only(
    work: &InboundInteractionWork,
    component: &ComponentId,
    database: &std::path::Path,
) -> Result<ConfirmationPlan, BusyComponentError> {
    let ComponentId::Busy { choice_id, action } = component else {
        return Err(BusyComponentError::ActionUnconfirmed);
    };
    let choice = authorized_choice(work, choice_id)?;
    validate_busy_choice(choice, *action, work.user_id.get(), work.channel_id.get())?;
    confirmation_if_ready(
        database,
        &busy_ready_marker(choice_id, work.user_id.get(), work.channel_id.get()),
        super::now()?,
        busy_confirmation_plan(choice_id),
    )
}

fn authorized_choice<'a>(
    work: &'a InboundInteractionWork,
    choice_id: &str,
) -> Result<&'a BusyChoice, BusyComponentError> {
    work.authorized_busy_choice
        .as_ref()
        .filter(|choice| choice.choice_id == choice_id)
        .ok_or(BusyComponentError::MissingAuthorizationSnapshot)
}

fn confirmation_if_ready(
    database: &std::path::Path,
    ready_marker: &str,
    now: f64,
    plan: ConfirmationPlan,
) -> Result<ConfirmationPlan, BusyComponentError> {
    if !confirmation_ready(database, ready_marker, now)? {
        return Err(BusyComponentError::ActionUnconfirmed);
    }
    Ok(plan)
}

pub fn validate_busy_choice(
    choice: &BusyChoice,
    action: BusyAction,
    user_id: u64,
    channel_id: u64,
) -> Result<(), BusyComponentError> {
    if choice.owner_user_id
        != i64::try_from(user_id).map_err(|_| BusyComponentError::IntegerRange)?
    {
        return Err(BusyComponentError::WrongUser);
    }
    if choice.channel_id
        != i64::try_from(channel_id).map_err(|_| BusyComponentError::IntegerRange)?
    {
        return Err(BusyComponentError::WrongChannel);
    }
    if action == BusyAction::Steer && cdr_pro::prompt::is_pro_command(&choice.prompt) {
        return Err(pro_steer_not_dispatched());
    }
    Ok(())
}

fn pro_steer_not_dispatched() -> BusyComponentError {
    BusyComponentError::ControlNotDispatched(
        "Pro requests cannot be steered; choose Queue next to run Pro connection checks".into(),
    )
}
