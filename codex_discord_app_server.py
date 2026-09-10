"""Discord-facing helpers for the resident Codex app-server transport."""

from __future__ import annotations

from collections.abc import Callable
from typing import Protocol, runtime_checkable

import codex_app_server_transport as app_server_transport
import codex_desktop_bridge as bridge
import codex_discord_app_server_admission as discord_app_server_admission
from codex_app_server_transport_delivery import AppServerDeliveryClient, BridgeModule
from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject


class AppServerClient(AppServerDeliveryClient, Protocol):
    def get_latest_pending_approval_request(self, thread_id: str) -> JsonObject | None: ...
    def get_latest_pending_input_request(self, thread_id: str) -> JsonObject | None: ...
    def reply_to_pending_approval(self, thread_id: str, answer_text: str) -> JsonObject: ...
    def reply_to_pending_input(self, thread_id: str, answer_text: str) -> JsonObject: ...


class ApprovalReplyClient(Protocol):
    def get_latest_pending_approval_request(self, thread_id: str) -> JsonObject | None: ...
    def reply_to_pending_approval(self, thread_id: str, answer_text: str) -> JsonObject: ...


class InputReplyClient(Protocol):
    def get_latest_pending_input_request(self, thread_id: str) -> JsonObject | None: ...
    def reply_to_pending_input(self, thread_id: str, answer_text: str) -> JsonObject: ...


class PendingInteractiveStateClient(Protocol):
    def get_latest_pending_approval_request(self, thread_id: str) -> JsonObject | None: ...
    def get_latest_pending_input_request(self, thread_id: str) -> JsonObject | None: ...


class AppServerTransportModule(Protocol):
    @property
    def DEFAULT_CLIENT(self) -> AppServerDeliveryClient: ...

    def start_turn_no_wait(
        self,
        client: AppServerDeliveryClient,
        prompt: str,
        target_thread_id: str | None,
        *,
        bridge_module: BridgeModule,
        confirm_timeout_sec: float,
        expected_generation: int | None = None,
    ) -> app_server_transport.AppServerDeliveryResult: ...

    def steer_or_start_no_wait(
        self,
        client: AppServerDeliveryClient,
        prompt: str,
        target_thread_id: str | None,
        *,
        bridge_module: BridgeModule,
        confirm_timeout_sec: float,
        expected_generation: int | None = None,
    ) -> app_server_transport.AppServerDeliveryResult: ...


@runtime_checkable
class AppServerStateLogger(Protocol):
    log_func: Callable[[str], None] | None


def _log_pending_state_check_failed(
    active_client: object,
    target_thread_id: str,
    kind: str,
    exc: Exception,
) -> None:
    if not isinstance(active_client, AppServerStateLogger):
        return
    log_func = active_client.log_func
    if log_func is None:
        return
    log_func(
        f"app_server_pending_interactive_state_check_failed thread={target_thread_id} "
        + f"kind={kind} error_type={type(exc).__name__} error={str(exc)[:300]}"
    )


def run_prompt_no_wait(
    prompt: str,
    target_thread_id: str | None,
    *,
    transport_module: AppServerTransportModule = app_server_transport,
    bridge_module: BridgeModule = bridge,
    client: AppServerDeliveryClient | None = None,
    confirm_timeout_sec: float = 6.0,
) -> tuple[int, str]:
    result = transport_module.start_turn_no_wait(
        client or transport_module.DEFAULT_CLIENT,
        prompt,
        target_thread_id,
        bridge_module=bridge_module,
        confirm_timeout_sec=confirm_timeout_sec,
        expected_generation=discord_app_server_admission.current_expected_app_server_generation(),
    )
    return result.exit_code, result.output


def run_steering_no_wait(
    prompt: str,
    target_thread_id: str | None,
    *,
    transport_module: AppServerTransportModule = app_server_transport,
    bridge_module: BridgeModule = bridge,
    client: AppServerDeliveryClient | None = None,
    confirm_timeout_sec: float,
) -> app_server_transport.AppServerDeliveryResult:
    return transport_module.steer_or_start_no_wait(
        client or transport_module.DEFAULT_CLIENT,
        prompt,
        target_thread_id,
        bridge_module=bridge_module,
        confirm_timeout_sec=confirm_timeout_sec,
        expected_generation=discord_app_server_admission.current_expected_app_server_generation(),
    )


def submit_approval_reply(
    target_thread_id: str,
    answer: str,
    *,
    client: ApprovalReplyClient | None = None,
) -> tuple[int, str] | None:
    active_client = client or app_server_transport.DEFAULT_CLIENT
    pending_request = active_client.get_latest_pending_approval_request(target_thread_id)
    if pending_request is None:
        return None
    try:
        result = active_client.reply_to_pending_approval(target_thread_id, answer)
    except (CodexAppServerTransportError, TimeoutError, OSError) as exc:
        return 1, f"ERROR: resident app-server approval writeback failed: {exc}"
    lines = [
        f"thread_id: {target_thread_id}",
        f"decision_action: {result.get('decision_action') or '-'}",
        f"request_kind: {result.get('request_kind') or '-'}",
        f"request_id: {result.get('request_id') or '-'}",
        "transport: resident-app-server approval",
    ]
    return 0, "\n".join(lines)


def submit_input_reply(
    target_thread_id: str,
    answer: str,
    *,
    client: InputReplyClient | None = None,
) -> tuple[int, str] | None:
    active_client = client or app_server_transport.DEFAULT_CLIENT
    pending_input = active_client.get_latest_pending_input_request(target_thread_id)
    if pending_input is None:
        return None
    try:
        result = active_client.reply_to_pending_input(target_thread_id, answer)
    except (CodexAppServerTransportError, TimeoutError, OSError) as exc:
        return 1, f"ERROR: resident app-server input writeback failed: {exc}"
    lines = [
        f"thread_id: {target_thread_id}",
        f"request_id: {result.get('request_id') or '-'}",
        "transport: resident-app-server input",
    ]
    answers_by_question = result.get("answers_by_question") or {}
    if isinstance(answers_by_question, dict):
        for question_id, values in answers_by_question.items():
            if isinstance(values, list):
                lines.append(f"{question_id}: {' | '.join(str(value) for value in values)}")
    return 0, "\n".join(lines)


def get_pending_interactive_state(
    target_thread_id: str,
    *,
    client: PendingInteractiveStateClient | None = None,
) -> str | None:
    active_client = client or app_server_transport.DEFAULT_CLIENT
    try:
        pending_request = active_client.get_latest_pending_approval_request(target_thread_id)
    except (CodexAppServerTransportError, TimeoutError, OSError) as exc:
        _log_pending_state_check_failed(active_client, target_thread_id, "approval", exc)
        pending_request = None
    if pending_request is not None:
        return "approval"
    try:
        pending_input = active_client.get_latest_pending_input_request(target_thread_id)
    except (CodexAppServerTransportError, TimeoutError, OSError) as exc:
        _log_pending_state_check_failed(active_client, target_thread_id, "input", exc)
        pending_input = None
    if pending_input is not None:
        return "input"
    return None
