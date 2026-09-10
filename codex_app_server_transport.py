"""Persistent Codex app-server transport for Discord delivery."""
# noqa: SIZE_OK — cohesive typed facade for the resident app-server protocol.

from __future__ import annotations

from typing import Final, final

from codex_app_server_transport_delivery import (
    AppServerDeliveryResult as AppServerDeliveryResult,
    start_turn_no_wait as start_turn_no_wait,
    steer_or_start_no_wait as steer_or_start_no_wait,
)
from codex_app_server_transport_replies import (
    CodexAppServerTransportError as CodexAppServerTransportError,
    JsonArray,
    JsonObject,
    build_approval_response as build_approval_response,
    build_input_response as build_input_response,
    parse_approval_answer as parse_approval_answer,
    resolve_input_answers as resolve_input_answers,
    split_input_values as split_input_values,
)
from codex_app_server_transport_resident import ResidentCodexAppServerTransport
from codex_app_server_transport_goal import (
    GoalAbsent as GoalAbsent,
    GoalPresent as GoalPresent,
    GoalTransportError as GoalTransportError,
    ThreadGoalStatus as ThreadGoalStatus,
    ThreadGoalUpdate as ThreadGoalUpdate,
    ThreadGoalLookup as ThreadGoalLookup,
    parse_thread_goal_status,
)
from codex_app_server_transport_lifecycle import (
    AppServerGenerationExpiredError as AppServerGenerationExpiredError,
    AppServerGenerationMismatch as AppServerGenerationMismatch,
    AppServerLifecycleSnapshot as AppServerLifecycleSnapshot,
    ChildCleanupRecycleOutcome as ChildCleanupRecycleOutcome,
    ChildCleanupRecycleStatus as ChildCleanupRecycleStatus,
)
from codex_app_server_transport_threads import (
    get_in_progress_turn_id as get_in_progress_turn_id,
    get_thread_status_type as get_thread_status_type,
)
from codex_app_server_transport_turn_outcomes import (
    InterruptOrigin as InterruptOrigin,
    TurnCompletion as TurnCompletion,
    TurnCompletionFound as TurnCompletionFound,
    TurnCompletionObservation as TurnCompletionObservation,
    TurnCompletionPending as TurnCompletionPending,
    TurnCompletionTransportError as TurnCompletionTransportError,
    TurnStatus as TurnStatus,
    parse_thread_turn_completions,
    parse_thread_turn_states,
)
from codex_app_server_transport_subscriptions import ThreadReleaseOutcome
from codex_pro_prompt_contract import build_turn_input


__all__ = [
    "AppServerDeliveryResult",
    "AppServerGenerationMismatch",
    "AppServerGenerationExpiredError",
    "AppServerLifecycleSnapshot",
    "ChildCleanupRecycleOutcome",
    "ChildCleanupRecycleStatus",
    "CodexAppServerTransportError",
    "PersistentCodexAppServer",
    "ThreadGoalStatus",
    "ThreadGoalLookup",
    "TurnCompletion",
    "TurnCompletionObservation",
    "TurnStatus",
    "build_approval_response",
    "build_input_response",
    "parse_approval_answer",
    "resolve_input_answers",
    "split_input_values",
]

INITIAL_RESUME_TIMEOUT_SEC: Final = 10.0


@final
class PersistentCodexAppServer(ResidentCodexAppServerTransport):
    def cancel_pending_server_requests(self, thread_id: str) -> int:
        with self._condition:
            requests = self._pending.pending_requests(thread_id)
        cancelled = 0
        for request in requests:
            raw_request_id = request["id"]
            request_id = str(raw_request_id)
            self._write_message(
                {
                    "id": raw_request_id,
                    "error": {
                        "code": -32800,
                        "message": "Request cancelled because the remote operation ended.",
                    },
                }
            )
            with self._condition:
                self._pending.resolve_request(request_id)
            self._log(
                f"app_server_request_cancelled_by_remote_cleanup id={request_id} target={thread_id}"
            )
            cancelled += 1
        if cancelled:
            self.notify_child_cleanup_blocker_changed()
        return cancelled

    def reply_to_pending_approval(self, thread_id: str, answer_text: str) -> JsonObject:
        request = self.get_latest_pending_approval_request(thread_id)
        if request is None:
            raise CodexAppServerTransportError("No pending app-server approval request for this thread.")
        request_id = str(request.get("id") or "").strip()
        method = str(request.get("method") or "").strip()
        params = request.get("params") or {}
        if not request_id:
            raise CodexAppServerTransportError("Pending app-server approval request had no id.")
        if not isinstance(params, dict):
            params = {}
        result, decision_action = build_approval_response(method, params, answer_text)
        self.respond_to_server_request(request_id, result)
        return {
            "request_id": request_id,
            "request_kind": method,
            "decision_action": decision_action,
            "verification_busy_state": "submitted",
        }

    def reply_to_pending_input(self, thread_id: str, answer_text: str) -> JsonObject:
        request = self.get_latest_pending_input_request(thread_id)
        if request is None:
            raise CodexAppServerTransportError("No pending app-server input request for this thread.")
        request_id = str(request.get("id") or "").strip()
        params = request.get("params") or {}
        if not request_id:
            raise CodexAppServerTransportError("Pending app-server input request had no id.")
        if not isinstance(params, dict):
            params = {}
        response_payload, answers_by_question = build_input_response(params, answer_text)
        self.respond_to_server_request(request_id, response_payload)
        answers_json: JsonObject = {}
        for question_id, values in answers_by_question.items():
            answer_values: JsonArray = []
            answer_values.extend(values)
            answers_json[question_id] = answer_values
        return {
            "request_id": request_id,
            "answers_by_question": answers_json,
            "verification_busy_state": "submitted",
        }

    def read_thread(
        self,
        thread_id: str,
        *,
        include_turns: bool = False,
        timeout_sec: float = 8.0,
        expected_generation: int | None = None,
        recovery_timeout_sec: float | None = None,
    ) -> JsonObject:
        params = {"threadId": thread_id, "includeTurns": include_turns}
        if expected_generation is None:
            return self.request(
                "thread/read",
                params,
                timeout_sec=timeout_sec,
                recovery_timeout_sec=recovery_timeout_sec,
            )
        return self.request(
            "thread/read",
            params,
            timeout_sec=timeout_sec,
            expected_generation=expected_generation,
            recovery_timeout_sec=recovery_timeout_sec,
        )

    def get_thread_goal_status(
        self,
        thread_id: str,
        *,
        timeout_sec: float = 8.0,
        expected_generation: int | None = None,
    ) -> ThreadGoalStatus | None:
        params = {"threadId": thread_id}
        if expected_generation is None:
            result = self.request("thread/goal/get", params, timeout_sec=timeout_sec)
        else:
            result = self.request(
                "thread/goal/get",
                params,
                timeout_sec=timeout_sec,
                expected_generation=expected_generation,
            )
        return parse_thread_goal_status(result, expected_thread_id=thread_id)

    def get_thread_goal_lookup(
        self,
        thread_id: str,
        *,
        timeout_sec: float = 3.0,
        expected_generation: int | None = None,
    ) -> ThreadGoalLookup:
        try:
            if expected_generation is None:
                status = self.get_thread_goal_status(thread_id, timeout_sec=timeout_sec)
            else:
                status = self.get_thread_goal_status(
                    thread_id,
                    timeout_sec=timeout_sec,
                    expected_generation=expected_generation,
                )
        except (CodexAppServerTransportError, OSError, TimeoutError) as exc:
            return GoalTransportError(f"{type(exc).__name__}: {str(exc)[:300]}")
        return GoalAbsent() if status is None else GoalPresent(status)

    def get_thread_turn_completions(
        self,
        thread_id: str,
        *,
        timeout_sec: float = 3.0,
    ) -> dict[str, TurnCompletion]:
        result = self.read_thread(thread_id, include_turns=True, timeout_sec=timeout_sec)
        completions = parse_thread_turn_completions(result, expected_thread_id=thread_id)
        for turn_id in list(completions):
            cached = self.get_cached_turn_completion(thread_id, turn_id)
            if cached is not None:
                completions[turn_id] = cached
        return completions

    def get_thread_turn_states(
        self,
        thread_id: str,
        *,
        timeout_sec: float = 3.0,
        expected_generation: int | None = None,
    ) -> dict[str, TurnCompletion]:
        if expected_generation is None:
            result = self.read_thread(thread_id, include_turns=True, timeout_sec=timeout_sec)
        else:
            result = self.read_thread(
                thread_id,
                include_turns=True,
                timeout_sec=timeout_sec,
                expected_generation=expected_generation,
            )
        states = parse_thread_turn_states(result, expected_thread_id=thread_id)
        for turn_id in list(states):
            cached = self.get_cached_turn_completion(thread_id, turn_id)
            if cached is not None:
                states[turn_id] = cached
        return states

    def resume_thread(
        self,
        thread_id: str,
        *,
        timeout_sec: float = INITIAL_RESUME_TIMEOUT_SEC,
        expected_generation: int | None = None,
    ) -> JsonObject:
        deadline = self.monotonic_func() + max(timeout_sec, 0.0)
        first_timeout = min(INITIAL_RESUME_TIMEOUT_SEC, max(timeout_sec, 0.0))
        try:
            if expected_generation is None:
                result = self.request("thread/resume", {"threadId": thread_id}, timeout_sec=first_timeout)
            else:
                result = self.request(
                    "thread/resume",
                    {"threadId": thread_id},
                    timeout_sec=first_timeout,
                    expected_generation=expected_generation,
                )
        except TimeoutError:
            remaining = max(0.0, deadline - self.monotonic_func())
            if remaining <= 0:
                raise
            self._log(
                f"app_server_thread_resume_retry thread={thread_id} "
                + f"first_timeout_sec={first_timeout:.1f} remaining_sec={remaining:.1f}"
            )
            if expected_generation is None:
                result = self.request("thread/resume", {"threadId": thread_id}, timeout_sec=remaining)
            else:
                result = self.request(
                    "thread/resume",
                    {"threadId": thread_id},
                    timeout_sec=remaining,
                    expected_generation=expected_generation,
                )
        self.mark_thread_subscribed(thread_id)
        return result

    def release_thread_subscription_if_terminal(
        self,
        thread_id: str,
        *,
        expected_generation: int | None = None,
    ) -> ThreadReleaseOutcome:
        outcome = self._subscriptions.release_if_terminal(
            self,
            thread_id,
            expected_generation=expected_generation,
            log=self._log,
        )
        if outcome.released:
            _ = self.try_recycle_child_cleanup(expected_generation=expected_generation)
        return outcome

    def start_turn(
        self,
        thread_id: str,
        prompt: str,
        *,
        expected_generation: int | None = None,
    ) -> JsonObject:
        return self.request(
            "turn/start",
            {
                "threadId": thread_id,
                "input": build_turn_input(prompt),
            },
            timeout_sec=12.0,
            expected_generation=expected_generation,
        )

    def steer_turn(
        self,
        thread_id: str,
        prompt: str,
        *,
        expected_turn_id: str,
        expected_generation: int | None = None,
    ) -> JsonObject:
        return self.request(
            "turn/steer",
            {
                "threadId": thread_id,
                "expectedTurnId": expected_turn_id,
                "input": build_turn_input(prompt),
            },
            timeout_sec=10.0,
            expected_generation=expected_generation,
        )

    def interrupt_turn(self, thread_id: str, turn_id: str) -> JsonObject:
        return self.request(
            "turn/interrupt",
            {"threadId": thread_id, "turnId": turn_id},
            timeout_sec=10.0,
        )

    def interrupt_turn_from_remote_user(self, thread_id: str, turn_id: str) -> JsonObject:
        registered = self.register_remote_interrupt_intent(thread_id, turn_id)
        try:
            return self.interrupt_turn(thread_id, turn_id)
        except Exception:  # noqa: BROAD_EXCEPT_OK - rollback must cover every interrupt failure.
            if registered:
                self.cancel_remote_interrupt_intent(thread_id, turn_id)
            raise

    def get_active_turn_id(
        self,
        thread_id: str,
        *,
        expected_generation: int | None = None,
    ) -> str | None:
        try:
            return self.get_active_turn_id_or_raise(thread_id, expected_generation=expected_generation)
        except (CodexAppServerTransportError, OSError, TimeoutError) as exc:
            self._log(
                f"app_server_active_turn_read_failed thread={thread_id} "
                + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
            )
            return None

    def has_active_turn_or_raise(
        self,
        thread_id: str,
        *,
        expected_generation: int | None = None,
    ) -> bool:
        with self._lock:
            if self._pending.active_turn_id(thread_id):
                return True
        if expected_generation is None:
            result = self.read_thread(thread_id, include_turns=False)
        else:
            result = self.read_thread(
                thread_id,
                include_turns=False,
                expected_generation=expected_generation,
            )
        payload = result.get("thread")
        if not isinstance(payload, dict):
            return False
        return get_thread_status_type(payload) == "active"

    def get_active_turn_id_or_raise(
        self,
        thread_id: str,
        *,
        expected_generation: int | None = None,
    ) -> str | None:
        with self._lock:
            turn_id = self._pending.active_turn_id(thread_id)
            if turn_id:
                return turn_id
        if expected_generation is None:
            payload = self.read_thread(thread_id, include_turns=True, timeout_sec=20.0).get("thread")
        else:
            payload = self.read_thread(
                thread_id,
                include_turns=True, timeout_sec=20.0,
                expected_generation=expected_generation,
            ).get("thread")
        if not isinstance(payload, dict):
            return None
        return get_in_progress_turn_id(payload)

DEFAULT_CLIENT = PersistentCodexAppServer()
