from __future__ import annotations

from codex_app_server_transport_approvals import (
    build_approval_response as build_approval_response,
    parse_approval_answer as parse_approval_answer,
)
from codex_app_server_transport_inputs import (
    build_input_response as build_input_response,
    resolve_input_answers as resolve_input_answers,
    split_input_values as split_input_values,
)
from codex_app_server_transport_reply_types import (
    CodexAppServerTransportError as CodexAppServerTransportError,
    JsonArray as JsonArray,
    JsonMapping as JsonMapping,
    JsonObject as JsonObject,
    JsonValue as JsonValue,
    Payload as Payload,
)


def extract_response_result(method: str, response: JsonMapping) -> JsonObject:
    if "error" in response:
        error = response.get("error") or {}
        if isinstance(error, dict):
            detail = str(error.get("message") or error)
        else:
            detail = str(error)
        raise CodexAppServerTransportError(f"{method} failed: {detail}")

    result = response.get("result") or {}
    if not isinstance(result, dict):
        raise CodexAppServerTransportError(f"{method} returned an invalid payload.")
    return result
