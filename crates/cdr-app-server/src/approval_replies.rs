use serde_json::{Value, json};

use crate::AppServerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalAnswer {
    pub decision: &'static str,
    pub legacy_decision: &'static str,
    pub scope: &'static str,
}

pub fn parse_approval_answer(answer: &str) -> Result<ApprovalAnswer, AppServerError> {
    let normalized = answer.trim();
    let lowered = normalized.to_lowercase();
    let parsed = if normalized == "1"
        || matches!(
            lowered.as_str(),
            "approve" | "approved" | "accept" | "yes" | "y" | "ok" | "예" | "네" | "승인"
        ) {
        ApprovalAnswer {
            decision: "accept",
            legacy_decision: "approved",
            scope: "turn",
        }
    } else if normalized == "2"
        || matches!(
            lowered.as_str(),
            "approve session" | "accept session" | "session" | "approve_for_session"
        )
    {
        ApprovalAnswer {
            decision: "acceptForSession",
            legacy_decision: "approved_for_session",
            scope: "session",
        }
    } else if normalized == "3"
        || matches!(
            lowered.as_str(),
            "decline" | "reject" | "no" | "n" | "아니요" | "거절"
        )
    {
        ApprovalAnswer {
            decision: "decline",
            legacy_decision: "denied",
            scope: "turn",
        }
    } else if matches!(
        lowered.as_str(),
        "cancel" | "skip" | "dismiss" | "건너뛰기" | "취소"
    ) {
        ApprovalAnswer {
            decision: "cancel",
            legacy_decision: "abort",
            scope: "turn",
        }
    } else {
        return Err(invalid(
            "Unrecognized approval reply. Use 1 to approve, 2 to approve for this session, 3 to decline, or cancel to skip.",
        ));
    };
    Ok(parsed)
}

pub fn build_approval_response(
    method: &str,
    params: &Value,
    answer: &str,
) -> Result<(Value, String), AppServerError> {
    let answer = parse_approval_answer(answer)?;
    let (payload, action) = match method {
        "mcpServer/elicitation/request" => {
            let action = if answer.decision == "acceptForSession" {
                "accept"
            } else {
                answer.decision
            };
            let content = if action == "accept" {
                json!({})
            } else {
                Value::Null
            };
            (
                json!({"action": action, "content": content, "_meta": null}),
                action,
            )
        }
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            (json!({"decision": answer.decision}), answer.decision)
        }
        "item/permissions/requestApproval"
            if matches!(answer.decision, "accept" | "acceptForSession") =>
        {
            let permissions = params
                .get("permissions")
                .filter(|value| value.is_object())
                .cloned()
                .unwrap_or_else(|| json!({"network": null, "fileSystem": null}));
            (
                json!({"permissions": permissions, "scope": answer.scope}),
                answer.decision,
            )
        }
        "item/permissions/requestApproval" => (
            json!({"permissions": {"network": null, "fileSystem": null}, "scope": "turn", "strictAutoReview": false}),
            answer.decision,
        ),
        "execCommandApproval" | "applyPatchApproval" => (
            json!({"decision": answer.legacy_decision}),
            answer.legacy_decision,
        ),
        _ => {
            return Err(invalid(format!(
                "Unsupported app-server approval request method: {method}"
            )));
        }
    };
    Ok((payload, action.to_owned()))
}

fn invalid(message: impl Into<String>) -> AppServerError {
    AppServerError::InvalidReply {
        message: message.into(),
    }
}
