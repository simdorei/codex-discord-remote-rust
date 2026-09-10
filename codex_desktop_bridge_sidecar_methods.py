from __future__ import annotations

from typing import Protocol

from codex_desktop_bridge_sidecar_protocol import CodexSidecarProtocolError
from codex_desktop_bridge_sidecar_types import JsonObject


class SidecarRequestClient(Protocol):
    def request(self, method: str, params: JsonObject, *, timeout_sec: float = 10.0) -> JsonObject: ...


def start_thread(client: SidecarRequestClient, cwd: str | None) -> JsonObject:
    params: JsonObject = {}
    if cwd:
        params["cwd"] = cwd
    return client.request("thread/start", params, timeout_sec=10.0)


def read_thread(client: SidecarRequestClient, thread_id: str, *, include_turns: bool = False) -> JsonObject:
    return request_thread(
        client,
        "thread/read",
        {"threadId": thread_id, "includeTurns": include_turns},
        timeout_sec=8.0,
    )


def resume_thread(client: SidecarRequestClient, thread_id: str) -> JsonObject:
    return request_thread(client, "thread/resume", {"threadId": thread_id}, timeout_sec=10.0)


def request_thread(
    client: SidecarRequestClient,
    method: str,
    params: JsonObject,
    *,
    timeout_sec: float,
) -> JsonObject:
    result = client.request(method, params, timeout_sec=timeout_sec)
    if not isinstance(result.get("thread"), dict):
        raise CodexSidecarProtocolError(f"{method} returned an invalid thread payload.")
    return result


def update_thread_settings(
    client: SidecarRequestClient,
    thread_id: str,
    settings: dict[str, str | None],
) -> JsonObject:
    params: JsonObject = {"threadId": thread_id}
    params.update(settings)
    return client.request("thread/settings/update", params, timeout_sec=10.0)


def list_models(client: SidecarRequestClient) -> JsonObject:
    return client.request("model/list", {}, timeout_sec=8.0)


def start_turn(client: SidecarRequestClient, thread_id: str, prompt: str) -> JsonObject:
    return client.request(
        "turn/start",
        {
            "threadId": thread_id,
            "input": [{"type": "text", "text": prompt, "text_elements": []}],
        },
        timeout_sec=12.0,
    )


def interrupt_turn(client: SidecarRequestClient, thread_id: str, turn_id: str) -> JsonObject:
    return client.request(
        "turn/interrupt",
        {"threadId": thread_id, "turnId": turn_id},
        timeout_sec=10.0,
    )


def clean_background_terminals(client: SidecarRequestClient, thread_id: str) -> JsonObject:
    return client.request(
        "thread/backgroundTerminals/clean",
        {"threadId": thread_id},
        timeout_sec=10.0,
    )


def archive_thread(client: SidecarRequestClient, thread_id: str) -> JsonObject:
    return client.request("thread/archive", {"threadId": thread_id}, timeout_sec=10.0)
