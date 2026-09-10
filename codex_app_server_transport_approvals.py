from __future__ import annotations

from codex_app_server_transport_reply_types import (
    CodexAppServerTransportError,
    JsonMapping,
    JsonValue,
    Payload,
)


def parse_approval_answer(answer_text: str) -> tuple[str, str, str]:
    normalized = str(answer_text).strip()
    lowered = normalized.lower()
    if normalized == "1" or lowered in {"approve", "approved", "accept", "yes", "y", "ok", "예", "네", "승인"}:
        return "accept", "approved", "turn"
    if normalized == "2" or lowered in {"approve session", "accept session", "session", "approve_for_session"}:
        return "acceptForSession", "approved_for_session", "session"
    if normalized == "3" or lowered in {"decline", "reject", "no", "n", "아니요", "거절"}:
        return "decline", "denied", "turn"
    if lowered in {"cancel", "skip", "dismiss", "건너뛰기", "취소"}:
        return "cancel", "abort", "turn"
    raise CodexAppServerTransportError(
        "Unrecognized approval reply. Use 1 to approve, 2 to approve for this session, "
        + "3 to decline, or cancel to skip."
    )


def build_approval_response(method: str, params: JsonMapping, answer_text: str) -> tuple[Payload, str]:
    decision, legacy_decision, scope = parse_approval_answer(answer_text)
    if method == "mcpServer/elicitation/request":
        action = "accept" if decision == "acceptForSession" else decision
        content: JsonValue = {} if action == "accept" else None
        return {"action": action, "content": content, "_meta": None}, action
    if method == "item/commandExecution/requestApproval":
        return {"decision": decision}, decision
    if method == "item/fileChange/requestApproval":
        if decision == "acceptForSession":
            return {"decision": "acceptForSession"}, decision
        return {"decision": decision}, decision
    if method == "item/permissions/requestApproval":
        if decision in {"accept", "acceptForSession"}:
            raw_permissions = params.get("permissions")
            permissions: JsonValue = (
                raw_permissions
                if isinstance(raw_permissions, dict)
                else {"network": None, "fileSystem": None}
            )
            return {"permissions": permissions, "scope": scope}, decision
        return {
            "permissions": {"network": None, "fileSystem": None},
            "scope": "turn",
            "strictAutoReview": False,
        }, decision
    if method in {"execCommandApproval", "applyPatchApproval"}:
        return {"decision": legacy_decision}, legacy_decision
    raise CodexAppServerTransportError(f"Unsupported app-server approval request method: {method}")
