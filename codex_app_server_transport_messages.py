from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
import json
from typing import Literal

from codex_app_server_transport_replies import JsonObject, JsonValue

type AppServerTransportLineKind = Literal[
    "empty",
    "invalid-json",
    "ignored",
    "server-request",
    "response",
    "notification",
]


@dataclass(frozen=True, slots=True)
class AppServerTransportLine:
    kind: AppServerTransportLineKind
    message: JsonObject | None = None
    message_id: str | None = None
    invalid_preview: str = ""


def classify_app_server_transport_line(
    raw_line: str,
    *,
    decode_json_value: Callable[[str], JsonValue],
) -> AppServerTransportLine:
    line = raw_line.strip()
    if not line:
        return AppServerTransportLine(kind="empty")
    try:
        parsed = decode_json_value(line)
    except json.JSONDecodeError:
        return AppServerTransportLine(kind="invalid-json", invalid_preview=line[:200])
    if not isinstance(parsed, dict):
        return AppServerTransportLine(kind="ignored")

    message = parsed
    raw_message_id = message.get("id")
    if raw_message_id is not None and "method" in message and "result" not in message and "error" not in message:
        return AppServerTransportLine(kind="server-request", message=message, message_id=str(raw_message_id))
    if raw_message_id is not None:
        return AppServerTransportLine(kind="response", message=message, message_id=str(raw_message_id))
    return AppServerTransportLine(kind="notification", message=message)
