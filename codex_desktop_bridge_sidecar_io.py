from __future__ import annotations

import json
import queue
import threading
import time
from collections.abc import Callable
from typing import TypeAlias

from codex_desktop_bridge_sidecar_process import SidecarProcess
from codex_desktop_bridge_sidecar_protocol import (
    CodexSidecarProcessExitedError,
    CodexSidecarProtocolError,
    decode_response_line,
    make_request_payload,
    response_result,
)
from codex_desktop_bridge_sidecar_types import JsonObject, JsonValue

ResponseQueue: TypeAlias = queue.Queue[str | None]
StdoutThread: TypeAlias = threading.Thread

_decode_json_value: Callable[[str], JsonValue] = json.loads


def new_response_queue() -> ResponseQueue:
    return queue.Queue()


def drain_stdout(process: SidecarProcess, stdout_queue: ResponseQueue) -> None:
    try:
        stdout = process.stdout
        if stdout is None:
            return
        for raw_line in stdout:
            stdout_queue.put(raw_line.rstrip("\r\n"))
    finally:
        stdout_queue.put(None)


def start_stdout_drain_thread(
    process: SidecarProcess,
    stdout_queue: ResponseQueue,
) -> threading.Thread:
    stdout_thread = threading.Thread(
        target=drain_stdout,
        args=(process, stdout_queue),
        daemon=True,
    )
    stdout_thread.start()
    return stdout_thread


def write_sidecar_request(
    process: SidecarProcess,
    request_id: str,
    method: str,
    params: JsonObject,
) -> None:
    stdin = process.stdin
    if stdin is None:
        raise CodexSidecarProtocolError("Codex app-server sidecar stdin is unavailable.")
    payload = make_request_payload(request_id, method, params)
    _ = stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
    _ = stdin.flush()


def read_sidecar_response(
    stdout_queue: ResponseQueue,
    request_id: str,
    method: str,
    timeout_sec: float,
) -> JsonObject:
    deadline = time.time() + max(timeout_sec, 0.0)
    while True:
        remaining = deadline - time.time()
        if remaining <= 0:
            raise TimeoutError(f"Timed out waiting for app-server response to {method}.")
        try:
            raw_line = stdout_queue.get(timeout=remaining)
        except queue.Empty as exc:
            raise TimeoutError(f"Timed out waiting for app-server response to {method}.") from exc

        if raw_line is None:
            raise CodexSidecarProcessExitedError(
                f"Codex app-server sidecar exited while waiting for {method}."
            )

        message = decode_response_line(
            raw_line,
            method,
            decode_json_value=_decode_json_value,
        )
        if message is None:
            continue
        if str(message.get("id") or "") != request_id:
            continue
        return response_result(method, message)
