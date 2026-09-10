from __future__ import annotations

from collections.abc import Mapping

from codex_app_server_transport_reply_types import (
    CodexAppServerTransportError,
    JsonArray,
    JsonMapping,
    JsonObject,
    Payload,
)


def split_input_values(raw_value: str) -> list[str]:
    values = [part.strip() for part in str(raw_value).split("|")]
    return [value for value in values if value]


def resolve_input_answers(question: JsonMapping, raw_value: str) -> list[str]:
    values = split_input_values(raw_value)
    if not values:
        raise CodexAppServerTransportError("Answer text was empty.")
    raw_options = question.get("options")
    options = raw_options if isinstance(raw_options, list) else []
    labels = [
        str(option.get("label") or "").strip()
        for option in options
        if isinstance(option, Mapping) and str(option.get("label") or "").strip()
    ]
    resolved: list[str] = []
    for value in values:
        if labels and value.isdigit():
            index = int(value) - 1
            if 0 <= index < len(labels):
                resolved.append(labels[index])
                continue
        matched = next((label for label in labels if label.lower() == value.lower()), None)
        resolved.append(matched or value)
    return resolved


def build_input_response(params: JsonMapping, answer_text: str) -> tuple[Payload, dict[str, list[str]]]:
    raw_questions = params.get("questions")
    questions = raw_questions if isinstance(raw_questions, list) else []
    if not questions:
        raise CodexAppServerTransportError("No pending input questions were available to answer.")
    question_map: dict[str, JsonMapping] = {}
    for index, question in enumerate(questions, start=1):
        if not isinstance(question, Mapping):
            raise CodexAppServerTransportError(f"Pending input question {index} was invalid.")
        question_id = str(question.get("id") or "").strip()
        if not question_id:
            raise CodexAppServerTransportError(f"Pending input question {index} did not include an id.")
        question_map[question_id] = question

    normalized = str(answer_text).strip()
    if not normalized:
        raise CodexAppServerTransportError("Answer text was empty.")

    answers_by_question: dict[str, list[str]] = {}
    if len(question_map) == 1 and "=" not in normalized:
        only_question_id = next(iter(question_map))
        answers_by_question[only_question_id] = resolve_input_answers(
            question_map[only_question_id],
            normalized,
        )
    else:
        assignments: dict[str, str] = {}
        for segment in normalized.split(";"):
            segment = segment.strip()
            if not segment:
                continue
            if "=" not in segment:
                raise CodexAppServerTransportError(
                    "Multi-question replies must use question_id=value; other_id=value format."
                )
            question_id, raw_value = segment.split("=", 1)
            question_id = question_id.strip()
            raw_value = raw_value.strip()
            if not question_id:
                raise CodexAppServerTransportError("A reply_input assignment was missing the question id.")
            assignments[question_id] = raw_value
        missing = [question_id for question_id in question_map if question_id not in assignments]
        if missing:
            raise CodexAppServerTransportError("Missing answers for question ids: " + ", ".join(missing))
        unknown = [question_id for question_id in assignments if question_id not in question_map]
        if unknown:
            raise CodexAppServerTransportError("Unknown question ids: " + ", ".join(unknown))
        for question_id, raw_value in assignments.items():
            answers_by_question[question_id] = resolve_input_answers(
                question_map[question_id],
                raw_value,
            )

    answers_payload: JsonObject = {}
    for question_id, values in answers_by_question.items():
        answer_values: JsonArray = []
        answer_values.extend(values)
        answers_payload[question_id] = {"answers": answer_values}
    return {"answers": answers_payload}, answers_by_question
