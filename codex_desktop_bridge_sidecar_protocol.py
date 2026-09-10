from __future__ import annotations

import json
from collections.abc import Callable

from codex_desktop_bridge_sidecar_types import JsonObject, JsonValue

_decode_json_value: Callable[[str], JsonValue] = json.loads


class CodexSidecarError(RuntimeError):
    pass


class CodexSidecarStartupError(RuntimeError):
    pass


class CodexSidecarProtocolError(RuntimeError):
    pass


class CodexSidecarProcessExitedError(RuntimeError):
    pass


def make_request_payload(request_id: str, method: str, params: JsonObject) -> JsonObject:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    }


def decode_response_line(
    raw_line: str,
    method: str,
    *,
    decode_json_value: Callable[[str], JsonValue] = _decode_json_value,
) -> JsonObject | None:
    line = raw_line.strip()
    if not line:
        return None
    try:
        value = decode_json_value(line)
    except json.JSONDecodeError as exc:
        raise CodexSidecarProtocolError(
            f"Codex app-server returned non-JSON output while waiting for {method}."
        ) from exc
    if not isinstance(value, dict):
        return None
    return value


def response_result(method: str, message: JsonObject) -> JsonObject:
    if "error" in message:
        raise CodexSidecarError(f"{method} failed: {_format_error_detail(message.get('error'))}")
    if "result" not in message:
        raise CodexSidecarProtocolError(f"{method} returned an invalid payload.")
    result = message.get("result")
    if not isinstance(result, dict):
        raise CodexSidecarProtocolError(f"{method} returned an invalid payload.")
    return result


def _format_error_detail(error: JsonValue | None) -> str:
    if isinstance(error, dict):
        message = error.get("message")
        return str(message or error)
    return str(error or {})
