use cdr_app_server::{RequestId, ServerRequest, extract_thread_id};
use cdr_discord::components::{ComponentError, ComponentRequestId, bound_approval_button_row};
use thiserror::Error;
use twilight_model::channel::message::Component;
mod input;
pub(crate) mod text_binding;
use input::input_prompt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerPrompt {
    pub thread_id: String,
    pub text: String,
    pub components: Vec<Component>,
}

#[derive(Debug, Error)]
pub enum ServerPromptError {
    #[error("app-server request has no thread id")]
    MissingThread,
    #[error("unsupported app-server request method: {0}")]
    Unsupported(String),
    #[error("app-server input request has no valid questions")]
    InvalidQuestions,
    #[error("Secret input must be completed in the Codex app, not in Discord")]
    SecretInput,
    #[error(transparent)]
    Component(#[from] ComponentError),
}

pub fn build_server_prompt(
    request: &ServerRequest,
    generation: u64,
) -> Result<ServerPrompt, ServerPromptError> {
    let thread_id = extract_thread_id(&request.params).ok_or(ServerPromptError::MissingThread)?;
    if is_approval(&request.method, &request.params) {
        return approval_prompt(request, thread_id, generation);
    }
    if request.method == "item/tool/requestUserInput" {
        return input_prompt(request, thread_id, generation);
    }
    Err(ServerPromptError::Unsupported(request.method.clone()))
}

fn approval_prompt(
    request: &ServerRequest,
    thread_id: String,
    generation: u64,
) -> Result<ServerPrompt, ServerPromptError> {
    let detail = ["reason", "command", "message"]
        .into_iter()
        .find_map(|key| safe_detail(request.params.get(key)))
        .unwrap_or_else(|| request.method.clone());
    Ok(ServerPrompt {
        components: vec![bound_approval_button_row(
            &thread_id,
            generation,
            request.occurrence.as_bytes(),
            component_request_id(&request.id),
        )?],
        text: format!(
            "Approval required\nthread: {thread_id}\nrequest: {}\ndetail: {detail}",
            request.method
        ),
        thread_id,
    })
}

fn component_request_id(request_id: &RequestId) -> ComponentRequestId<'_> {
    match request_id {
        RequestId::String(value) => ComponentRequestId::String(value),
        RequestId::Integer(value) => ComponentRequestId::Integer(*value),
    }
}

fn is_approval(method: &str, params: &serde_json::Value) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "execCommandApproval"
            | "applyPatchApproval"
    ) || (method == "mcpServer/elicitation/request"
        && params.get("mode").and_then(serde_json::Value::as_str) == Some("url"))
}

fn safe_detail(value: Option<&serde_json::Value>) -> Option<String> {
    let text = match value? {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let bounded = text.trim().chars().take(1_000).collect::<String>();
    (!bounded.is_empty()).then_some(bounded)
}
