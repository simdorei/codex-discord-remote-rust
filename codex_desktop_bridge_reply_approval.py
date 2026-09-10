from __future__ import annotations

from pathlib import Path
from typing import NoReturn

from codex_desktop_bridge_reply_types import JsonObject, JsonValue, PendingReplyDeps, ReplyResult, ReplyThread


class PendingApprovalSnapshotMissingError(RuntimeError):
    pass


class PendingApprovalMissingError(RuntimeError):
    pass


class PendingApprovalRequestIdMissingError(RuntimeError):
    pass


class PendingApprovalRequestKindMissingError(RuntimeError):
    pass


def reply_to_pending_approval(
    thread: ReplyThread,
    answer_text: str,
    timeout_sec: float,
    deps: PendingReplyDeps,
) -> ReplyResult:
    pending_request = deps.get_pending_approval_request(thread, timeout_sec)
    if pending_request is None:
        busy_state = deps.get_thread_busy_state(thread, True)
        if busy_state == "waiting-approval":
            pending_request = deps.get_cached_live_approval_request(thread.id)
    if pending_request is None:
        permission_result = _try_permission_approval_fallback(thread, answer_text, timeout_sec, deps)
        if permission_result is not None:
            return permission_result

        busy_state = deps.get_thread_busy_state(thread, True)
        if busy_state == "waiting-approval":
            raise PendingApprovalSnapshotMissingError(
                "The thread still looks like waiting-approval, but no pending approval request snapshot "
                + "was received over IPC. Open the thread once in Codex Desktop and retry."
            )
        raise PendingApprovalMissingError("No pending approval request is active for the selected thread.")

    raw_request_id = pending_request.get("request_id")
    if raw_request_id is None:
        raise PendingApprovalRequestIdMissingError("The pending approval request did not include a request id.")
    request_id = _clean_string(raw_request_id)
    if not request_id:
        raise PendingApprovalRequestIdMissingError("The pending approval request did not include a request id.")

    request_kind = _clean_string(pending_request.get("request_kind"))
    if not request_kind:
        raise PendingApprovalRequestKindMissingError("The pending approval request did not include a request kind.")

    return _submit_pending_approval_request(
        thread=thread,
        answer_text=answer_text,
        timeout_sec=timeout_sec,
        pending_request=pending_request,
        raw_request_id=raw_request_id,
        request_id=request_id,
        request_kind=request_kind,
        deps=deps,
    )


def _try_permission_approval_fallback(
    thread: ReplyThread,
    answer_text: str,
    timeout_sec: float,
    deps: PendingReplyDeps,
) -> ReplyResult | None:
    session_path = Path(thread.rollout_path)
    permission_prompt = deps.get_pending_permission_approval_from_session(session_path)
    if permission_prompt is None:
        return None

    result = deps.submit_permission_approval_via_ui_row_select(answer_text)
    deadline = deps.time_now() + max(timeout_sec, 8.0)
    last_state = "waiting-approval"
    while deps.time_now() < deadline:
        deps.sleep(0.5)
        last_state = deps.get_thread_busy_state(thread, True)
        if last_state != "waiting-approval":
            result["verification_busy_state"] = last_state
            result["request_id"] = permission_prompt.get("call_id") or ""
            return result
    raise RuntimeError(
        "Permission approval UI action ran, but the thread is still waiting-approval.\n"
        + f"thread: {thread.id}\n"
        + f"call_id: {permission_prompt.get('call_id') or '-'}\n"
        + f"tool: {permission_prompt.get('tool_name') or '-'}\n"
        + f"verification_busy_state: {last_state}"
    )


def _submit_pending_approval_request(
    *,
    thread: ReplyThread,
    answer_text: str,
    timeout_sec: float,
    pending_request: JsonObject,
    raw_request_id: JsonValue,
    request_id: str,
    request_kind: str,
    deps: PendingReplyDeps,
) -> ReplyResult:
    decision_payload, decision_action = deps.build_approval_decision_payload(answer_text)
    last_result: ReplyResult | None = None
    attempts: list[str] = []
    payload_candidates = deps.build_approval_decision_candidate_payloads(decision_payload)

    for payload_index, candidate_payload in enumerate(payload_candidates, start=1):
        for use_target_client in _approval_target_modes(pending_request):
            result = deps.submit_approval_decision(
                thread,
                raw_request_id,
                candidate_payload,
                request_kind,
                timeout_sec,
                use_target_client,
            )
            last_result = result
            attempts.append(_approval_attempt_label(payload_index, candidate_payload, use_target_client))
            deps.sleep(0.75)
            busy_state = deps.get_thread_busy_state(thread, True)
            if busy_state != "waiting-approval":
                return _approval_success_result(
                    result,
                    thread=thread,
                    request_id=request_id,
                    decision_action=decision_action,
                    request_kind=request_kind,
                    busy_state=busy_state,
                    attempts=attempts,
                    deps=deps,
                )

    _raise_still_waiting_approval_failure(thread, request_id, request_kind, attempts, last_result)


def _approval_target_modes(pending_request: JsonObject) -> tuple[bool, ...]:
    if _clean_string(pending_request.get("owner_client_id")):
        return (True, False)
    return (True,)


def _approval_attempt_label(payload_index: int, candidate_payload: JsonValue, use_target_client: bool) -> str:
    payload_kind = "string" if isinstance(candidate_payload, str) else "object"
    target = "owner" if use_target_client else "broadcast"
    return f"payload#{payload_index}={payload_kind}/target={target}"


def _approval_success_result(
    result: ReplyResult,
    *,
    thread: ReplyThread,
    request_id: str,
    decision_action: str,
    request_kind: str,
    busy_state: str,
    attempts: list[str],
    deps: PendingReplyDeps,
) -> ReplyResult:
    deps.clear_cached_live_approval_request(thread.id)
    _ = result.setdefault("request_id", request_id)
    result["decision_action"] = decision_action
    result["request_kind"] = request_kind
    result["verification_busy_state"] = busy_state
    result["attempts"] = _json_string_list(attempts)
    return result


def _raise_still_waiting_approval_failure(
    thread: ReplyThread,
    request_id: str,
    request_kind: str,
    attempts: list[str],
    last_result: ReplyResult | None,
) -> NoReturn:
    failure_lines = [
        "Approval submit was acknowledged, but the thread is still waiting-approval.",
        f"thread: {thread.id}",
        f"request_id: {request_id}",
        f"request_kind: {request_kind}",
    ]
    if attempts:
        failure_lines.extend(["", "attempts:"])
        failure_lines.extend(f"- {entry}" for entry in attempts)
    if last_result:
        handled_by = _clean_string(last_result.get("owner_client_id"))
        if handled_by:
            failure_lines.extend(["", f"handled_by_client: {handled_by}"])
    raise RuntimeError("\n".join(failure_lines))


def _clean_string(value: JsonValue | None) -> str:
    return value.strip() if isinstance(value, str) else ""


def _json_string_list(values: list[str]) -> list[JsonValue]:
    result: list[JsonValue] = []
    for value in values:
        result.append(value)
    return result
