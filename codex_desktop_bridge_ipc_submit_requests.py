from __future__ import annotations

from typing import Final, TypeAlias

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]

APPROVAL_DECISION_METHODS: Final[dict[str, str]] = {
    "commandExecution": "thread-follower-command-approval-decision",
    "fileChange": "thread-follower-file-approval-decision",
}


class UnsupportedApprovalRequestKindError(RuntimeError):
    def __init__(self, request_kind: str) -> None:
        self.request_kind: str = request_kind
        super().__init__(f"Unsupported approval request kind: {request_kind}")


def approval_decision_method_for_request_kind(request_kind: str) -> str:
    method = APPROVAL_DECISION_METHODS.get(request_kind)
    if method:
        return method
    raise UnsupportedApprovalRequestKindError(request_kind)


def build_submit_user_input_request(
    *,
    ipc_request_id: str,
    source_client_id: str,
    thread_id: str,
    request_id: str,
    response_payload: JsonObject,
    owner_client_id: str | None,
) -> JsonObject:
    request: JsonObject = {
        "type": "request",
        "requestId": ipc_request_id,
        "sourceClientId": source_client_id,
        "version": 1,
        "method": "thread-follower-submit-user-input",
        "params": {
            "conversationId": thread_id,
            "requestId": request_id,
            "response": response_payload,
        },
    }
    if owner_client_id:
        request["targetClientId"] = owner_client_id
    return request


def build_submit_approval_decision_request(
    *,
    ipc_request_id: str,
    source_client_id: str,
    thread_id: str,
    request_id: JsonValue,
    decision_payload: JsonValue,
    owner_client_id: str | None,
    method: str,
    use_target_client: bool,
) -> JsonObject:
    request: JsonObject = {
        "type": "request",
        "requestId": ipc_request_id,
        "sourceClientId": source_client_id,
        "version": 1,
        "method": method,
        "params": {
            "conversationId": thread_id,
            "requestId": request_id,
            "decision": decision_payload,
        },
    }
    if use_target_client and owner_client_id:
        request["targetClientId"] = owner_client_id
    return request
