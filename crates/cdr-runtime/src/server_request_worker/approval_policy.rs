//! The operator's explicit all-approval policy, applied after original authority
//! verification and before connector-specific handling.
use crate::config::ApprovalPolicy;
use cdr_app_server::{AppServerError, ServerRequest, build_approval_response};
use serde_json::Value;

pub(super) fn response(
    request: &ServerRequest,
    policy: ApprovalPolicy,
) -> Result<Option<Value>, AppServerError> {
    if policy != ApprovalPolicy::Allow {
        return Ok(None);
    }
    let approval = matches!(
        request.method.as_str(),
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "execCommandApproval"
            | "applyPatchApproval"
    ) || (request.method == "mcpServer/elicitation/request"
        && match request.params.get("mode").and_then(Value::as_str) {
            Some("url") => true,
            Some("form" | "openai/form") => {
                super::local_tools::empty_approval_schema(request.params.get("requestedSchema"))
            }
            _ => false,
        });
    if !approval {
        // Questions and forms requiring values are not approval decisions.
        return Ok(None);
    }
    // Use the same protocol payload as the owner's Allow button. Each request
    // is accepted once; no permanent native grant or invented input is added.
    build_approval_response(&request.method, &request.params, "1")
        .map(|(response, _)| Some(response))
}
