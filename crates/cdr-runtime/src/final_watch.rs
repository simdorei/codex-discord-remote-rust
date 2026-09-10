use std::collections::BTreeSet;

use cdr_app_server::goal::ThreadGoalStatus;
use cdr_app_server::outcomes::{InterruptOrigin, TurnCompletion, TurnStatus};

mod events;

pub use events::process_events;

const NO_VISIBLE_REPLY: &str = "Codex turn completed without a visible reply.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalLookup {
    Absent,
    Present(ThreadGoalStatus),
    TransportError(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeObservation {
    Pending,
    Found(TurnCompletion),
    TransportError(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchStatus {
    Final,
    Progress,
    Aborted,
    Failed,
    TransportError,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchResult {
    pub status: WatchStatus,
    pub commentary: Vec<String>,
    pub final_answer: String,
    pub error_message: String,
    pub interrupt_origin: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FinalWatchState {
    pub commentary: Vec<String>,
    pub(super) final_candidate: String,
    pub(super) seen_agent_messages: BTreeSet<String>,
    pub(super) seen_interactive_notices: BTreeSet<String>,
    pub(super) rollout_terminal: Option<RolloutTerminal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RolloutTerminal {
    pub(super) kind: String,
    pub(super) last_agent_message: String,
}

pub fn reconcile_native(
    state: &mut FinalWatchState,
    observation: &NativeObservation,
    goal: &GoalLookup,
    reconciliation_elapsed: bool,
) -> Option<WatchResult> {
    match observation {
        NativeObservation::TransportError(message) => Some(result(
            state,
            WatchStatus::TransportError,
            "",
            message,
            None,
        )),
        NativeObservation::Pending => (reconciliation_elapsed && state.rollout_terminal.is_some())
            .then(|| {
                result(
                    state,
                    WatchStatus::TransportError,
                    "",
                    "Timed out reconciling the Codex terminal state.",
                    None,
                )
            }),
        NativeObservation::Found(completion) => match completion.status {
            TurnStatus::InProgress => None,
            TurnStatus::Interrupted => Some(result(
                state,
                WatchStatus::Aborted,
                "",
                "",
                completion.interrupt_origin.map(interrupt_name),
            )),
            TurnStatus::Failed => Some(result(
                state,
                WatchStatus::Failed,
                "",
                if completion.error_message.is_empty() {
                    "Codex turn failed without an error message."
                } else {
                    &completion.error_message
                },
                None,
            )),
            TurnStatus::Completed => complete_native(state, goal, reconciliation_elapsed),
        },
    }
}

fn complete_native(
    state: &mut FinalWatchState,
    goal: &GoalLookup,
    reconciliation_elapsed: bool,
) -> Option<WatchResult> {
    let exact = state
        .rollout_terminal
        .as_ref()
        .filter(|terminal| terminal.kind == "task_complete")
        .map(|terminal| terminal.last_agent_message.clone());
    if exact.is_none() && !reconciliation_elapsed {
        return None;
    }
    Some(complete_for_goal(
        state,
        exact
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| NO_VISIBLE_REPLY.into()),
        goal,
    ))
}

pub(super) fn complete_for_goal(
    state: &mut FinalWatchState,
    answer: String,
    goal: &GoalLookup,
) -> WatchResult {
    match goal {
        GoalLookup::TransportError(message) => {
            result(state, WatchStatus::TransportError, "", message, None)
        }
        GoalLookup::Present(status) if *status != ThreadGoalStatus::Complete => {
            events::append_commentary(state, answer, true);
            result(state, WatchStatus::Progress, "", "", None)
        }
        GoalLookup::Absent | GoalLookup::Present(_) => {
            result(state, WatchStatus::Final, &answer, "", None)
        }
    }
}

pub(super) fn result(
    state: &FinalWatchState,
    status: WatchStatus,
    answer: &str,
    error: &str,
    interrupt_origin: Option<&str>,
) -> WatchResult {
    WatchResult {
        status,
        commentary: state.commentary.clone(),
        final_answer: answer.into(),
        error_message: error.into(),
        interrupt_origin: interrupt_origin.map(str::to_owned),
    }
}

const fn interrupt_name(origin: InterruptOrigin) -> &'static str {
    match origin {
        InterruptOrigin::RemoteUserIntent => "remote_user_intent",
        InterruptOrigin::ExternalOrUnknown => "external_or_unknown",
    }
}
