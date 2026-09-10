use super::BusyComponentError;
use crate::{action_executor::ActionExecutor, queue_runner::TurnBackend};
use cdr_app_server::{
    ResidentAppServer,
    requests::{interrupt_turn, steer_turn},
};
use cdr_discord::components::BusyAction;
use cdr_store::claims::BusyChoice;

pub(super) async fn execute_busy_action<B: TurnBackend>(
    choice: &BusyChoice,
    action: BusyAction,
    server: &ResidentAppServer,
    database: &std::path::Path,
    executor: &ActionExecutor<B>,
) -> Result<(), BusyComponentError> {
    if action == BusyAction::Ignore {
        return Ok(());
    }
    if action == BusyAction::Steer && cdr_pro::prompt::is_pro_command(&choice.prompt) {
        return Err(super::pro_steer_not_dispatched());
    }
    let thread_id = choice
        .target_thread_id
        .as_deref()
        .ok_or(BusyComponentError::NoTarget)?;
    let _control = executor
        .control_lock(thread_id)
        .await
        .map_err(not_dispatched)?;
    let expected = cdr_store::control_binding::resolve(database,&choice.choice_id,thread_id)
        .map_err(not_dispatched)?
        .ok_or_else(|| BusyComponentError::ControlNotDispatched("this choice has no confirmed original turn yet; retry when its preceding task starts, or send a new request".into()))?;
    let channel = u64::try_from(choice.channel_id).map_err(not_dispatched)?;
    let (turn_id, generation) = executor
        .verified_control_turn(channel, thread_id, Some(&expected))
        .await
        .map_err(not_dispatched)?;
    match action {
        BusyAction::Steer => {
            cdr_store::mirror::record_user_origin(
                database,
                thread_id,
                &turn_id,
                &choice.prompt,
                crate::component_worker::now().map_err(not_dispatched)?,
            )
            .map_err(not_dispatched)?;
            server
                .execute(
                    steer_turn(thread_id, &choice.prompt, &turn_id),
                    Some(generation),
                )
                .await?;
            Ok(())
        }
        BusyAction::Stop => {
            server
                .execute(interrupt_turn(thread_id, &turn_id), Some(generation))
                .await?;
            Ok(())
        }
        BusyAction::Queue | BusyAction::Ignore => unreachable!(),
    }
}

// Only used before the first control RPC. Errors after dispatch keep their
// original certainty so an unreadable/lost reply cannot authorize a resend.
fn not_dispatched(error: impl std::fmt::Display) -> BusyComponentError {
    BusyComponentError::ControlNotDispatched(error.to_string())
}
