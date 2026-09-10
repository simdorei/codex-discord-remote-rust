from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

from codex_desktop_bridge_sidecar import CodexAppServerSidecar
from codex_desktop_bridge_sidecar_protocol import CodexSidecarProtocolError
from codex_desktop_bridge_sidecar_types import JsonObject
from codex_thread_models import ThreadInfo


class LoadableSidecarClient(Protocol):
    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> JsonObject: ...

    def resume_thread(self, thread_id: str) -> JsonObject: ...


class SidecarTurnClient(LoadableSidecarClient, Protocol):
    def start_turn(self, thread_id: str, prompt: str) -> JsonObject: ...

    def close(self) -> None: ...


EnsureThreadLoaded = Callable[[SidecarTurnClient, str], JsonObject]
IsTransientSidecarAttachError = Callable[[Exception], bool]
NewSidecar = Callable[[], SidecarTurnClient]
Sleep = Callable[[float], None]
StartTurnResult = dict[str, str | SidecarTurnClient]
TimeNow = Callable[[], float]


@dataclass(frozen=True, slots=True)
class StartTurnSidecarDeps:
    new_sidecar: NewSidecar
    ensure_thread_loaded: EnsureThreadLoaded
    is_transient_sidecar_attach_error: IsTransientSidecarAttachError
    time_now: TimeNow
    sleep: Sleep


def get_sidecar_thread_status_type(thread_payload: JsonObject) -> str:
    status = thread_payload.get("status") or {}
    if isinstance(status, dict):
        return str(status.get("type") or "").strip()
    return ""


def ensure_thread_loaded_via_sidecar(client: LoadableSidecarClient, thread_id: str) -> JsonObject:
    thread_payload = _thread_payload_from_response(client.read_thread(thread_id, include_turns=False))
    if get_sidecar_thread_status_type(thread_payload) != "notLoaded":
        return thread_payload
    resumed_thread = _thread_payload_from_response(client.resume_thread(thread_id))
    if not resumed_thread:
        raise CodexSidecarProtocolError("thread/resume did not return a thread payload.")
    return resumed_thread


def get_in_progress_turn_id(thread_payload: JsonObject) -> str | None:
    turns = thread_payload.get("turns") or []
    if not isinstance(turns, list):
        return None
    for turn in reversed(turns):
        if not isinstance(turn, dict):
            continue
        turn_id = str(turn.get("id") or "").strip()
        status = str(turn.get("status") or "").strip()
        if turn_id and status == "inProgress":
            return turn_id
    return None


def interrupt_thread_via_sidecar(thread: ThreadInfo) -> bool:
    with CodexAppServerSidecar() as client:
        _ = ensure_thread_loaded_via_sidecar(client, thread.id)
        thread_payload = _thread_payload_from_response(client.read_thread(thread.id, include_turns=True))
        turn_id = get_in_progress_turn_id(thread_payload)
        if turn_id:
            _ = client.interrupt_turn(thread.id, turn_id)
            return True
        _ = client.clean_background_terminals(thread.id)
        return True


def get_active_turn_id_via_app_server(thread_id: str) -> str | None:
    import codex_app_server_transport as app_server_transport

    return app_server_transport.DEFAULT_CLIENT.get_active_turn_id(thread_id)


def get_active_turn_id_via_app_server_or_raise(thread_id: str) -> str | None:
    import codex_app_server_transport as app_server_transport

    return app_server_transport.DEFAULT_CLIENT.get_active_turn_id_or_raise(thread_id)


def interrupt_turn_via_app_server(thread_id: str, turn_id: str) -> object:
    import codex_app_server_transport as app_server_transport
    import codex_discord_interrupt_context as discord_interrupt_context

    if discord_interrupt_context.is_discord_remote_stop():
        return app_server_transport.DEFAULT_CLIENT.interrupt_turn_from_remote_user(thread_id, turn_id)
    return app_server_transport.DEFAULT_CLIENT.interrupt_turn(thread_id, turn_id)


def is_transient_sidecar_attach_error(exc: Exception) -> bool:
    detail = str(exc).lower()
    return "thread not found" in detail or "no rollout found" in detail


def start_turn_via_sidecar(
    thread: ThreadInfo,
    prompt: str,
    *,
    timeout_sec: float = 10.0,
    keep_client_open: bool = False,
    deps: StartTurnSidecarDeps | None = None,
) -> StartTurnResult:
    active_deps = deps or StartTurnSidecarDeps(
        new_sidecar=CodexAppServerSidecar,
        ensure_thread_loaded=ensure_thread_loaded_via_sidecar,
        is_transient_sidecar_attach_error=is_transient_sidecar_attach_error,
        time_now=time.time,
        sleep=time.sleep,
    )
    deadline = active_deps.time_now() + max(timeout_sec, 0.0)
    attempt = 0
    last_error = ""
    while True:
        attempt += 1
        client: SidecarTurnClient | None = None
        keep_open = False
        try:
            client = active_deps.new_sidecar()
            _ = active_deps.ensure_thread_loaded(client, thread.id)
            result = client.start_turn(thread.id, prompt)
            turn_value = result.get("turn")
            turn_id = ""
            if isinstance(turn_value, dict):
                turn_id = str(turn_value.get("id") or "").strip()
            payload: StartTurnResult = {
                "owner_client_id": "",
                "turn_id": turn_id,
                "attempts": str(attempt),
            }
            if keep_client_open:
                payload["_client"] = client
                keep_open = True
            return payload
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK - sidecar attach retry boundary.
            last_error = str(exc)
            if active_deps.time_now() >= deadline or not active_deps.is_transient_sidecar_attach_error(exc):
                message = (
                    f"Local sidecar could not attach to the selected thread in time. Last error: {last_error}"
                )
                raise RuntimeError(message) from exc
            active_deps.sleep(min(0.5 * attempt, 1.5))
        finally:
            if client is not None and not keep_open:
                client.close()


def _thread_payload_from_response(response: JsonObject) -> JsonObject:
    thread_payload = response.get("thread")
    if isinstance(thread_payload, dict):
        return thread_payload
    return {}
