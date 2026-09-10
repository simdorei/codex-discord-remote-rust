from __future__ import annotations

from collections import deque
import time
from typing import Callable, Final, final

from codex_app_server_transport_goal import ThreadGoalUpdate, parse_thread_goal_update
from codex_app_server_transport_replies import JsonMapping, JsonObject
from codex_app_server_transport_threads import extract_thread_id, extract_turn_id
from codex_app_server_transport_turn_outcomes import (
    TurnCompletion,
    parse_turn_completion_notification,
)


LogFunc = Callable[[str], None]

APPROVAL_REQUEST_METHODS: Final = frozenset(
    {
        "item/commandExecution/requestApproval",
        "item/fileChange/requestApproval",
        "item/permissions/requestApproval",
        "execCommandApproval",
        "applyPatchApproval",
    }
)
INPUT_REQUEST_METHOD: Final = "item/tool/requestUserInput"
MCP_ELICITATION_REQUEST_METHOD: Final = "mcpServer/elicitation/request"
REMOTE_INTERRUPT_INTENT_TTL_SEC: Final = 60.0
REMOTE_INTERRUPT_INTENT_MAX: Final = 500


def _params(message: JsonObject) -> JsonMapping | None:
    params = message.get("params")
    return params if isinstance(params, dict) else None


def _is_approval_request(request: JsonObject) -> bool:
    method = str(request.get("method") or "")
    if method in APPROVAL_REQUEST_METHODS:
        return True
    params = _params(request)
    return method == MCP_ELICITATION_REQUEST_METHOD and params is not None and params.get("mode") == "url"


@final
class PendingRequestState:
    __slots__ = (
        "active_turns",
        "notifications",
        "remote_interrupt_intents",
        "server_request_order",
        "server_requests",
        "turn_completions",
        "goal_updates",
    )

    def __init__(self) -> None:
        self.notifications: deque[JsonObject] = deque(maxlen=1000)
        self.active_turns: dict[str, str] = {}
        self.turn_completions: deque[TurnCompletion] = deque(maxlen=1000)
        self.goal_updates: deque[ThreadGoalUpdate] = deque(maxlen=1000)
        self.remote_interrupt_intents: dict[tuple[str, str], float] = {}
        self.server_requests: dict[str, JsonObject] = {}
        self.server_request_order: deque[str] = deque(maxlen=500)

    def clear(self) -> None:
        self.notifications.clear()
        self.active_turns.clear()
        self.turn_completions.clear()
        self.goal_updates.clear()
        self.remote_interrupt_intents.clear()
        self.server_requests.clear()
        self.server_request_order.clear()

    def record_server_request(self, request_id: str, message: JsonObject, log: LogFunc) -> None:
        self.server_requests[request_id] = message
        self.server_request_order.append(request_id)
        method = str(message.get("method") or "")
        params = _params(message)
        thread_id = extract_thread_id(params) if params is not None else ""
        log(
            f"app_server_request_pending id={request_id} method={method or '-'} "
            + f"target={thread_id or '-'}"
        )

    def record_notification(
        self,
        message: JsonObject,
        log: LogFunc,
        *,
        now: float | None = None,
    ) -> None:
        self.notifications.append(message)
        method = str(message.get("method") or "")
        params = _params(message)
        if params is None:
            return
        if method == "turn/started":
            thread_id = extract_thread_id(params)
            turn_id = extract_turn_id(params)
            if thread_id and turn_id:
                self.active_turns[thread_id] = turn_id
                log(f"app_server_turn_started target={thread_id} turn={turn_id}")
        elif method == "turn/completed":
            thread_id = extract_thread_id(params)
            turn_id = extract_turn_id(params)
            intent_key = (thread_id, turn_id)
            self._prune_remote_interrupt_intents(time.monotonic() if now is None else now)
            completion = parse_turn_completion_notification(
                params,
                remote_user_intent=intent_key in self.remote_interrupt_intents,
            )
            _ = self.remote_interrupt_intents.pop(intent_key, None)
            if completion is not None:
                self.turn_completions.append(completion)
            if thread_id and self.active_turns.get(thread_id) == turn_id:
                _ = self.active_turns.pop(thread_id, None)
            if thread_id:
                status = completion.status.value if completion is not None else "unknown"
                origin = completion.interrupt_origin.value if completion and completion.interrupt_origin else "-"
                log(
                    f"app_server_turn_completed target={thread_id} turn={turn_id or '-'} "
                    + f"status={status} interrupt_origin={origin}"
                )
        elif method == "thread/goal/updated":
            update = parse_thread_goal_update(params)
            self.goal_updates.append(update)
            log(
                f"app_server_goal_updated target={update.thread_id} "
                + f"turn={update.turn_id or '-'} status={update.status.value}"
            )

    def resolve_request(self, request_id: str) -> None:
        _ = self.server_requests.pop(request_id, None)

    def pending_requests(self, thread_id: str | None = None) -> list[JsonObject]:
        requests = [
            self.server_requests[request_id]
            for request_id in self.server_request_order
            if request_id in self.server_requests
        ]
        if not thread_id:
            return list(requests)
        result: list[JsonObject] = []
        for request in requests:
            params = _params(request)
            if params is not None and extract_thread_id(params) == thread_id:
                result.append(request)
        return result

    def latest_approval_request(self, thread_id: str) -> JsonObject | None:
        for request in reversed(self.pending_requests(thread_id)):
            if _is_approval_request(request):
                return request
        return None

    def latest_input_request(self, thread_id: str) -> JsonObject | None:
        for request in reversed(self.pending_requests(thread_id)):
            if str(request.get("method") or "") == INPUT_REQUEST_METHOD:
                return request
        return None

    def active_turn_id(self, thread_id: str) -> str | None:
        return self.active_turns.get(thread_id)

    def has_active_turns(self) -> bool:
        return bool(self.active_turns)

    def reconcile_inactive_thread(
        self,
        thread_id: str,
        expected_turn_id: str,
        log: LogFunc,
    ) -> bool:
        if self.active_turns.get(thread_id) != expected_turn_id:
            return False
        _ = self.active_turns.pop(thread_id, None)
        _ = self.remote_interrupt_intents.pop((thread_id, expected_turn_id), None)
        log(
            "app_server_active_turn_reconciled_from_thread_read "
            + f"target={thread_id} turn={expected_turn_id}"
        )
        return True

    def has_pending_server_requests(self) -> bool:
        return bool(self.server_requests)

    def turn_completion(self, thread_id: str, turn_id: str) -> TurnCompletion | None:
        for completion in reversed(self.turn_completions):
            if completion.thread_id == thread_id and completion.turn_id == turn_id:
                return completion
        return None

    def turn_completions_for_thread(self, thread_id: str) -> dict[str, TurnCompletion]:
        return {
            completion.turn_id: completion
            for completion in self.turn_completions
            if completion.thread_id == thread_id
        }

    def register_remote_interrupt_intent(
        self,
        thread_id: str,
        turn_id: str,
        *,
        registered_at: float | None = None,
    ) -> bool:
        key = (thread_id, turn_id)
        current = time.monotonic() if registered_at is None else registered_at
        self._prune_remote_interrupt_intents(current)
        if self.active_turns.get(thread_id) != turn_id or self.turn_completion(thread_id, turn_id) is not None:
            return False
        if key in self.remote_interrupt_intents:
            return True
        if len(self.remote_interrupt_intents) >= REMOTE_INTERRUPT_INTENT_MAX:
            oldest = next(iter(self.remote_interrupt_intents))
            _ = self.remote_interrupt_intents.pop(oldest, None)
        self.remote_interrupt_intents[key] = current
        return True

    def cancel_remote_interrupt_intent(self, thread_id: str, turn_id: str) -> None:
        _ = self.remote_interrupt_intents.pop((thread_id, turn_id), None)

    def goal_update(self, thread_id: str, turn_id: str) -> ThreadGoalUpdate | None:
        for update in reversed(self.goal_updates):
            if update.thread_id == thread_id and update.turn_id == turn_id:
                return update
        return None

    def _prune_remote_interrupt_intents(self, current: float) -> None:
        expired = [
            key
            for key, registered_at in self.remote_interrupt_intents.items()
            if current - registered_at > REMOTE_INTERRUPT_INTENT_TTL_SEC
        ]
        for key in expired:
            _ = self.remote_interrupt_intents.pop(key, None)
