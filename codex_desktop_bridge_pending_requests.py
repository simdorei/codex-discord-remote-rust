from __future__ import annotations

from typing import TypeAlias

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]


def extract_pending_approval_request(conversation_state: JsonObject, thread_id: str) -> JsonObject | None:
    requests = conversation_state.get("requests") or []
    if not isinstance(requests, list):
        return None

    kind_by_method = {
        "item/commandExecution/requestApproval": "commandExecution",
        "item/fileChange/requestApproval": "fileChange",
    }
    for request in reversed(requests):
        if not isinstance(request, dict):
            continue
        method = clean_request_string(request.get("method"))
        request_kind = kind_by_method.get(method)
        if request_kind is None:
            continue
        params = request.get("params") or {}
        if not isinstance(params, dict):
            continue
        request_thread_id = clean_request_string(params.get("threadId") or thread_id)
        if request_thread_id and request_thread_id != thread_id:
            continue
        raw_request_id = request.get("id")
        if raw_request_id is None:
            continue
        request_id = str(raw_request_id).strip()
        if not request_id:
            continue
        return {
            "thread_id": thread_id,
            "request_id": raw_request_id,
            "request_kind": request_kind,
            "method": method,
            "item_id": clean_request_string(params.get("itemId")),
            "reason": clean_request_string(params.get("reason")),
        }
    return None


def extract_pending_user_input_request(conversation_state: JsonObject, thread_id: str) -> JsonObject | None:
    requests = conversation_state.get("requests") or []
    if not isinstance(requests, list):
        return None

    for request in reversed(requests):
        if not isinstance(request, dict):
            continue
        method = clean_request_string(request.get("method"))
        if method != "item/tool/requestUserInput":
            continue
        params = request.get("params") or {}
        if not isinstance(params, dict):
            continue
        request_thread_id = clean_request_string(params.get("threadId") or thread_id)
        if request_thread_id and request_thread_id != thread_id:
            continue
        questions = _extract_user_input_questions(params.get("questions") or [])
        if not questions:
            continue

        return {
            "thread_id": thread_id,
            "request_id": clean_request_string(request.get("id")),
            "turn_id": clean_request_string(params.get("turnId")),
            "item_id": clean_request_string(params.get("itemId")),
            "questions": questions,
        }

    return None


def clean_request_string(value: JsonValue) -> str:
    return value.strip() if isinstance(value, str) else ""


def _extract_user_input_questions(questions_payload: JsonValue) -> list[JsonValue]:
    if not isinstance(questions_payload, list) or not questions_payload:
        return []

    questions: list[JsonValue] = []
    for question in questions_payload:
        if not isinstance(question, dict):
            continue
        options = _extract_input_options(question.get("options") or [])
        questions.append(
            {
                "id": clean_request_string(question.get("id")),
                "header": clean_request_string(question.get("header")),
                "question": clean_request_string(question.get("question")),
                "is_other": question.get("isOther") is True,
                "is_secret": question.get("isSecret") is True,
                "options": options,
            }
        )
    return questions


def _extract_input_options(options_payload: JsonValue) -> list[JsonValue]:
    options: list[JsonValue] = []
    if not isinstance(options_payload, list):
        return options
    for option in options_payload:
        if not isinstance(option, dict):
            continue
        label = clean_request_string(option.get("label"))
        description = clean_request_string(option.get("description"))
        if not label:
            continue
        options.append({"label": label, "description": description})
    return options
