from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from threading import Lock
from typing import Final, Generic, Protocol, TypeVar

import codex_pro_session_mirror_gate as pro_session_mirror_gate
from codex_pro_browser_evidence import ProChromeUnavailableError
from codex_pro_prompt_contract import is_pro_skill_prompt


class PromptRelay(Protocol):
    def feed_line(self, line: str) -> None: ...

    def finish(self) -> None: ...


class PromptDeliveryResult(Protocol):
    @property
    def exit_code(self) -> int: ...

    @property
    def output(self) -> str: ...

    @property
    def thread_id(self) -> str | None: ...

    @property
    def turn_id(self) -> str | None: ...

    @property
    def target_ref(self) -> str: ...

    @property
    def session_path(self) -> str | None: ...

    @property
    def start_offset(self) -> int | None: ...

    @property
    def delivery_pending(self) -> bool: ...


RelayT = TypeVar("RelayT", bound=PromptRelay)
RelayContraT = TypeVar("RelayContraT", bound=PromptRelay, contravariant=True)
DeliveryResultT = TypeVar("DeliveryResultT", bound=PromptDeliveryResult)
SteeringResultT = TypeVar("SteeringResultT")
PromptNoWait = Callable[[str, str | None], tuple[int, str]]
TransportEnabled = Callable[[], bool]
PrepareProBrowserSession = Callable[[str | None], None]
CompleteProBrowserSession = Callable[[str | None, str | None], None]
StartTurnNoWait = Callable[[str, str | None], DeliveryResultT]
MakeSteeringResult = Callable[[DeliveryResultT], SteeringResultT]
WatchStream = Callable[[SteeringResultT, RelayT], tuple[int, str]]
LogFunc = Callable[[str], None]


_PRO_BROWSER_TURN_LOCK: Final = Lock()


class LegacyAskStream(Protocol[RelayContraT]):
    def __call__(
        self,
        prompt: str,
        relay: RelayContraT,
        *,
        force_while_busy: bool = False,
        wait: bool = True,
        target_thread_id: str | None = None,
    ) -> tuple[int, str]: ...


@dataclass(frozen=True, slots=True)
class PromptTransportDeps(Generic[RelayT, DeliveryResultT, SteeringResultT]):
    app_server_transport_enabled: TransportEnabled
    prepare_pro_browser_session: PrepareProBrowserSession
    complete_pro_browser_session: CompleteProBrowserSession
    run_resident_prompt_no_wait: PromptNoWait
    run_legacy_prompt_no_wait: PromptNoWait
    start_turn_no_wait: StartTurnNoWait[DeliveryResultT]
    make_steering_prompt_result: MakeSteeringResult[DeliveryResultT, SteeringResultT]
    run_watch_stream: WatchStream[SteeringResultT, RelayT]
    run_legacy_stream: LegacyAskStream[RelayT]
    log: LogFunc


def _transport_error_output(exc: Exception) -> str:
    if isinstance(exc, ProChromeUnavailableError):
        return str(exc)
    message = str(exc)
    lines = [f"ERROR: resident app-server transport failed: {message}"]
    if "Thread not found:" in message:
        lines.extend(
            [
                "",
                "The mapped Codex thread exists in local history, but the resident app-server cannot open it.",
                "Run `!mirror sync`; it refreshes app-server thread availability and removes stale mirror mappings.",
            ]
        )
    if isinstance(exc, TimeoutError) and "thread/resume" in message:
        lines.extend(
            [
                "",
                "A large conversation history or temporary PC load can delay restoring this Codex thread.",
                "Run `!resume` to retry the restore, then resend the original message.",
                "The failed prompt was not resent automatically.",
            ]
        )
    if isinstance(exc, TimeoutError) and "thread/read" in message:
        lines.extend(
            [
                "",
                "Only this thread status check timed out. This timeout did not stop or restart other Codex threads.",
                "Run `!resume` to check this thread again, then resend the original message.",
                "The failed prompt was not sent.",
            ]
        )
    return "\n".join(lines)


def _pro_transport_error_output(exc: Exception) -> str:
    if isinstance(exc, ProChromeUnavailableError):
        return str(exc)
    return f"ERROR: resident app-server Pro transport failed: {exc}"


def _log_transport_failure(log: LogFunc, *, event: str, target_thread_id: str | None, exc: Exception) -> None:
    log(f"{event} target={target_thread_id or '-'} " + f"error_type={type(exc).__name__} error={str(exc)[:300]}")


def _run_pro_prompt(
    prompt: str,
    target_thread_id: str | None,
    deps: PromptTransportDeps[RelayT, DeliveryResultT, SteeringResultT],
) -> DeliveryResultT:
    with _PRO_BROWSER_TURN_LOCK:
        pro_session_mirror_gate.hold(target_thread_id)
        try:
            deps.prepare_pro_browser_session(target_thread_id)
            result = deps.start_turn_no_wait(prompt, target_thread_id)
            if result.exit_code == 0:
                deps.complete_pro_browser_session(
                    target_thread_id,
                    result.turn_id,
                )
        except Exception:  # noqa: BROAD_EXCEPT_OK - mirror gate must close for every transport failure.
            pro_session_mirror_gate.reject(target_thread_id)
            raise
        if result.exit_code == 0:
            pro_session_mirror_gate.approve(target_thread_id)
        else:
            pro_session_mirror_gate.reject(target_thread_id)
        return result


def run_transport_prompt_no_wait(
    prompt: str,
    target_thread_id: str | None,
    deps: PromptTransportDeps[RelayT, DeliveryResultT, SteeringResultT],
) -> tuple[int, str]:
    if not pro_session_mirror_gate.wait_until_open(target_thread_id):
        gate_error = pro_session_mirror_gate.ProSessionMirrorGateTimeoutError(
            target_thread_id
        )
        _log_transport_failure(
            deps.log,
            event="pro_session_mirror_gate_timeout",
            target_thread_id=target_thread_id,
            exc=gate_error,
        )
        return 1, _transport_error_output(gate_error)
    pro_prompt = is_pro_skill_prompt(prompt)
    if pro_prompt:
        try:
            result = _run_pro_prompt(prompt, target_thread_id, deps)
            return result.exit_code, result.output
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK - transport boundary surfaces Pro failure.
            _log_transport_failure(
                deps.log,
                event="pro_app_server_prompt_failed",
                target_thread_id=target_thread_id,
                exc=exc,
            )
            return 1, _pro_transport_error_output(exc)
    if not deps.app_server_transport_enabled():
        return deps.run_legacy_prompt_no_wait(prompt, target_thread_id)
    try:
        return deps.run_resident_prompt_no_wait(prompt, target_thread_id)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK - transport boundary surfaces resident failure.
        _log_transport_failure(
            deps.log,
            event="app_server_prompt_failed",
            target_thread_id=target_thread_id,
            exc=exc,
        )
        return 1, _transport_error_output(exc)


def run_ask_stream(
    prompt: str,
    relay: RelayT,
    *,
    force_while_busy: bool = False,
    wait: bool = True,
    target_thread_id: str | None = None,
    deps: PromptTransportDeps[RelayT, DeliveryResultT, SteeringResultT],
) -> tuple[int, str]:
    if not pro_session_mirror_gate.wait_until_open(target_thread_id):
        gate_error = pro_session_mirror_gate.ProSessionMirrorGateTimeoutError(
            target_thread_id
        )
        _log_transport_failure(
            deps.log,
            event="pro_session_mirror_gate_timeout",
            target_thread_id=target_thread_id,
            exc=gate_error,
        )
        relay.finish()
        return 1, _transport_error_output(gate_error)
    pro_prompt = is_pro_skill_prompt(prompt)
    if pro_prompt:
        try:
            result = _run_pro_prompt(prompt, target_thread_id, deps)
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK - transport boundary surfaces Pro failure.
            _log_transport_failure(
                deps.log,
                event="pro_app_server_stream_prompt_failed",
                target_thread_id=target_thread_id,
                exc=exc,
            )
            relay.finish()
            return 1, _pro_transport_error_output(exc)
        if result.exit_code == 0 and result.session_path and result.start_offset is not None:
            return deps.run_watch_stream(deps.make_steering_prompt_result(result), relay)
        relay.finish()
        return result.exit_code, result.output
    if not deps.app_server_transport_enabled():
        return deps.run_legacy_stream(
            prompt,
            relay,
            force_while_busy=force_while_busy,
            wait=wait,
            target_thread_id=target_thread_id,
        )
    try:
        result = deps.start_turn_no_wait(prompt, target_thread_id)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK - transport boundary surfaces resident failure.
        _log_transport_failure(
            deps.log,
            event="app_server_stream_prompt_failed",
            target_thread_id=target_thread_id,
            exc=exc,
        )
        relay.finish()
        return 1, _transport_error_output(exc)
    if result.exit_code == 0 and wait and result.session_path and result.start_offset is not None:
        return deps.run_watch_stream(deps.make_steering_prompt_result(result), relay)
    relay.finish()
    return result.exit_code, result.output
