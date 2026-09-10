from __future__ import annotations

from collections.abc import Callable, Mapping
from typing import Protocol, TypeAlias

from codex_app_server_transport_delivery import AppServerDeliveryClient, BridgeModule
import codex_discord_app_server as discord_app_server
import codex_discord_steering as discord_steering
from codex_session_events import JsonValue
from codex_thread_models import ThreadInfo

ExceptionTypes: TypeAlias = tuple[type[BaseException], ...]
ResolveTargetRefFunc: TypeAlias = Callable[[str | None], tuple[str | None, str]]
RunBridgeCommandFunc: TypeAlias = Callable[[list[str]], tuple[int, str]]


class AppServerSteeringResult(Protocol):
    exit_code: int
    output: str
    thread_id: str | None
    target_ref: str
    session_path: str | None
    start_offset: int | None
    delivery_pending: bool
    turn_id: str | None


class RunSteeringNoWaitFunc(Protocol):
    def __call__(
        self,
        prompt: str,
        target_thread_id: str | None,
        *,
        transport_module: discord_app_server.AppServerTransportModule,
        bridge_module: BridgeModule,
        client: AppServerDeliveryClient,
        confirm_timeout_sec: float,
    ) -> AppServerSteeringResult: ...


class SubmitReplyFunc(Protocol):
    def __call__(
        self,
        target_thread_id: str,
        answer: str,
        *,
        client: discord_app_server.AppServerClient | None = None,
    ) -> tuple[int, str] | None: ...


class PendingInputReplyBridge(Protocol):
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo: ...

    def reply_to_pending_user_input(
        self,
        thread: ThreadInfo,
        answer_text: str,
        timeout_sec: float = 6.0,
    ) -> Mapping[str, JsonValue]: ...

    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str: ...


def run_resident_app_server_steering_prompt(
    prompt: str,
    target_thread_id: str | None,
    *,
    resolve_target_ref_func: ResolveTargetRefFunc,
    run_steering_no_wait_func: RunSteeringNoWaitFunc,
    transport_module: discord_app_server.AppServerTransportModule,
    bridge_module: BridgeModule,
    client: AppServerDeliveryClient,
    get_confirm_timeout_func: Callable[[], float],
    expected_exceptions: ExceptionTypes,
    log_func: Callable[[str], None],
) -> discord_steering.SteeringPromptResult:
    resolved_thread_id, target_ref = resolve_target_ref_func(target_thread_id)
    try:
        result = run_steering_no_wait_func(
            prompt,
            resolved_thread_id,
            transport_module=transport_module,
            bridge_module=bridge_module,
            client=client,
            confirm_timeout_sec=get_confirm_timeout_func(),
        )
    except expected_exceptions as exc:
        log_func(
            f"app_server_steering_failed target={resolved_thread_id or '-'} "
            + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
        )
        return discord_steering.SteeringPromptResult(
            1,
            f"ERROR: resident app-server transport failed: {exc}",
            target_thread_id=resolved_thread_id,
            target_ref=target_ref or "-",
        )
    watch_target: discord_steering.WatchTarget = (
        discord_steering.NativeExactWatchTarget(result.turn_id)
        if result.turn_id
        else discord_steering.RolloutOnlyWatchTarget()
    )
    return discord_steering.SteeringPromptResult(
        result.exit_code,
        result.output,
        target_thread_id=result.thread_id,
        target_ref=result.target_ref,
        session_path=result.session_path,
        start_offset=result.start_offset,
        delivery_pending=result.delivery_pending,
        watch_target=watch_target,
    )


def submit_approval_reply(
    target_thread_id: str,
    answer: str,
    *,
    app_server_transport_enabled_func: Callable[[], bool],
    submit_reply_func: SubmitReplyFunc,
    client: discord_app_server.AppServerClient,
    run_bridge_command_func: RunBridgeCommandFunc,
) -> tuple[int, str]:
    if app_server_transport_enabled_func():
        app_server_result = submit_reply_func(target_thread_id, answer, client=client)
        if app_server_result is not None:
            return app_server_result
    return run_bridge_command_func(["approval_reply", answer, target_thread_id])


def submit_input_reply(
    target_thread_id: str,
    answer: str,
    *,
    app_server_transport_enabled_func: Callable[[], bool],
    submit_reply_func: SubmitReplyFunc,
    client: discord_app_server.AppServerClient,
    pending_input_bridge: PendingInputReplyBridge,
) -> tuple[int, str]:
    if app_server_transport_enabled_func():
        app_server_result = submit_reply_func(target_thread_id, answer, client=client)
        if app_server_result is not None:
            return app_server_result
    try:
        thread = pending_input_bridge.choose_thread(target_thread_id, None)
        result = pending_input_bridge.reply_to_pending_user_input(thread, answer, timeout_sec=8.0)
        answers_by_question = result.get("answers_by_question") or {}
        lines = [
            f"thread_id: {thread.id}",
            f"thread_ref: {pending_input_bridge.get_thread_workspace_ref(thread)}",
        ]
        if isinstance(answers_by_question, dict):
            for question_id, values in answers_by_question.items():
                if isinstance(values, list):
                    lines.append(f"{question_id}: {' | '.join(str(value) for value in values)}")
        return 0, "\n".join(lines)
    except (OSError, RuntimeError, ValueError) as exc:
        return 1, f"ERROR: {exc}"


__all__ = [
    "PendingInputReplyBridge",
    "RunSteeringNoWaitFunc",
    "SubmitReplyFunc",
    "run_resident_app_server_steering_prompt",
    "submit_approval_reply",
    "submit_input_reply",
]
