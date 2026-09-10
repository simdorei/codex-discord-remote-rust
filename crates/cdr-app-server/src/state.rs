use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use crate::{AppServerError, Notification, RequestId, ServerRequest, ServerRequestOccurrence};

const MAX_NOTIFICATIONS: usize = 1_000;

mod dead_generation;
mod server_requests;
mod settings;

use server_requests::ServerRequestState;
pub(crate) use server_requests::{ServerRequestRecordError, ServerRequestRecordOutcome};

pub const APPROVAL_REQUEST_METHODS: &[&str] = &[
    "item/commandExecution/requestApproval",
    "item/fileChange/requestApproval",
    "item/permissions/requestApproval",
    "execCommandApproval",
    "applyPatchApproval",
];
pub const INPUT_REQUEST_METHOD: &str = "item/tool/requestUserInput";
pub const MCP_ELICITATION_REQUEST_METHOD: &str = "mcpServer/elicitation/request";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleSnapshot {
    pub generation: u64,
    pub healthy: bool,
    pub initialized: bool,
    pub process_id: Option<u32>,
    pub closed_reason: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct RuntimeState {
    pub generation: u64,
    pub initialized: bool,
    pub process_id: Option<u32>,
    pub closed_reason: Option<String>,
    active_turns: HashMap<String, String>,
    notifications: VecDeque<Notification>,
    notification_revision: u64,
    server_requests: ServerRequestState,
}

impl RuntimeState {
    pub(crate) fn starting(process_id: Option<u32>) -> Self {
        Self {
            process_id,
            ..Self::default()
        }
    }

    pub(crate) fn snapshot(&self) -> LifecycleSnapshot {
        LifecycleSnapshot {
            generation: self.generation,
            healthy: self.initialized && self.closed_reason.is_none() && self.process_id.is_some(),
            initialized: self.initialized,
            process_id: self.process_id,
            closed_reason: self.closed_reason.clone(),
        }
    }

    pub(crate) fn record_notification(&mut self, notification: Notification) {
        self.notification_revision = self.notification_revision.saturating_add(1);
        if notification.method == "turn/started" {
            if let (Some(thread_id), Some(turn_id)) = (
                extract_thread_id(&notification.params),
                extract_turn_id(&notification.params),
            ) {
                self.active_turns.insert(thread_id, turn_id);
            }
        } else if notification.method == "turn/completed"
            && let (Some(thread_id), Some(turn_id)) = (
                extract_thread_id(&notification.params),
                extract_turn_id(&notification.params),
            )
            && self.active_turns.get(&thread_id) == Some(&turn_id)
        {
            self.active_turns.remove(&thread_id);
        }
        self.notifications.push_back(notification);
        if self.notifications.len() > MAX_NOTIFICATIONS {
            self.notifications.pop_front();
        }
    }

    pub(crate) fn record_server_request(
        &mut self,
        request: ServerRequest,
    ) -> Result<ServerRequestRecordOutcome, ServerRequestRecordError> {
        self.server_requests.record(request)
    }

    pub(crate) fn begin_server_response(
        &mut self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) -> Result<(), AppServerError> {
        self.server_requests.begin_response(id, occurrence)
    }

    pub(crate) fn server_response_candidate(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) -> Result<&ServerRequest, AppServerError> {
        self.server_requests.response_candidate(id, occurrence)
    }

    pub(crate) fn mark_server_response_indeterminate(
        &mut self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) {
        self.server_requests.mark_indeterminate(id, occurrence);
    }

    pub(crate) fn resolve_server_request(
        &mut self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) -> Result<Option<ServerRequest>, AppServerError> {
        self.server_requests.resolve(id, occurrence)
    }

    pub(crate) fn pending_server_requests(&self, thread_id: Option<&str>) -> Vec<ServerRequest> {
        self.server_requests.pending(thread_id)
    }

    pub(crate) fn unsettled_server_requests(&self, thread_id: Option<&str>) -> Vec<ServerRequest> {
        self.server_requests.unsettled(thread_id)
    }

    pub(crate) fn has_unsettled_server_requests(&self) -> bool {
        self.server_requests.has_unsettled()
    }

    pub(crate) fn latest_approval_request(&self, thread_id: &str) -> Option<ServerRequest> {
        self.pending_server_requests(Some(thread_id))
            .into_iter()
            .rev()
            .find(is_approval_request)
    }

    pub(crate) fn latest_input_request(&self, thread_id: &str) -> Option<ServerRequest> {
        self.pending_server_requests(Some(thread_id))
            .into_iter()
            .rev()
            .find(|request| request.method == INPUT_REQUEST_METHOD)
    }

    pub(crate) fn active_turn_id(&self, thread_id: &str) -> Option<String> {
        self.active_turns.get(thread_id).cloned()
    }

    pub(crate) fn has_active_turns(&self) -> bool {
        !self.active_turns.is_empty()
    }
}

fn is_approval_request(request: &ServerRequest) -> bool {
    APPROVAL_REQUEST_METHODS.contains(&request.method.as_str())
        || (request.method == MCP_ELICITATION_REQUEST_METHOD
            && request.params.get("mode").and_then(Value::as_str) == Some("url"))
}

#[must_use]
pub fn extract_thread_id(params: &Value) -> Option<String> {
    for key in ["threadId", "conversationId"] {
        if let Some(value) = nonempty(params.get(key)) {
            return Some(value);
        }
    }
    params
        .get("thread")
        .and_then(|thread| nonempty(thread.get("id")))
        .or_else(|| {
            params.get("turn").and_then(|turn| {
                nonempty(turn.get("threadId")).or_else(|| nonempty(turn.get("conversationId")))
            })
        })
}

#[must_use]
pub fn extract_turn_id(params: &Value) -> Option<String> {
    for key in ["turnId", "id"] {
        if let Some(value) = nonempty(params.get(key)) {
            return Some(value);
        }
    }
    params.get("turn").and_then(|turn| nonempty(turn.get("id")))
}

fn nonempty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
