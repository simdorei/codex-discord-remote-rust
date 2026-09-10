from __future__ import annotations

from typing import TypeAlias

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]


class ReplyInputAnswerEmptyError(RuntimeError):
    pass


class ReplyInputQuestionsMissingError(RuntimeError):
    pass


class ReplyInputQuestionIdMissingError(RuntimeError):
    pass


class ReplyInputAssignmentFormatError(RuntimeError):
    pass


class ReplyInputAssignmentQuestionIdMissingError(RuntimeError):
    pass


class ReplyInputMissingAnswersError(RuntimeError):
    pass


class ReplyInputUnknownQuestionError(RuntimeError):
    pass


class ApprovalReplyUnrecognizedError(RuntimeError):
    pass


def split_reply_input_values(raw_value: str) -> list[str]:
    values = [part.strip() for part in str(raw_value).split("|")]
    return [value for value in values if value]


def resolve_reply_input_answers(question: JsonObject, raw_value: str) -> list[str]:
    values = split_reply_input_values(raw_value)
    if not values:
        raise ReplyInputAnswerEmptyError("Answer text was empty.")
    option_labels = _option_labels(question)
    resolved: list[str] = []
    for value in values:
        if option_labels and value.isdigit():
            option_index = int(value) - 1
            if 0 <= option_index < len(option_labels):
                resolved.append(option_labels[option_index])
                continue
        matched_label = next(
            (label for label in option_labels if label.lower() == value.lower()),
            None,
        )
        resolved.append(matched_label if matched_label is not None else value)
    return resolved


def build_reply_input_response_payload(
    pending_request: JsonObject,
    answer_text: str,
) -> tuple[JsonObject, dict[str, list[str]]]:
    questions = _question_map(pending_request)
    normalized = answer_text.strip()
    if not normalized:
        raise ReplyInputAnswerEmptyError("Answer text was empty.")

    answers_by_question: dict[str, list[str]] = {}
    if len(questions) == 1 and "=" not in normalized:
        only_question_id = next(iter(questions))
        answers_by_question[only_question_id] = resolve_reply_input_answers(
            questions[only_question_id],
            normalized,
        )
    else:
        assignments = _parse_multi_question_assignments(normalized)
        _require_matching_question_ids(questions, assignments)
        for question_id, raw_value in assignments.items():
            answers_by_question[question_id] = resolve_reply_input_answers(
                questions[question_id],
                raw_value,
            )

    answers_payload: JsonObject = {}
    for question_id, values in answers_by_question.items():
        answer_values: list[JsonValue] = [value for value in values]
        answers_payload[question_id] = {"answers": answer_values}
    response_payload: JsonObject = {"answers": answers_payload}
    return response_payload, answers_by_question


def build_approval_decision_payload(answer_text: str) -> tuple[str, str]:
    normalized = str(answer_text).strip()
    lowered = normalized.lower()
    if normalized == "1" or lowered in {
        "approve",
        "approved",
        "accept",
        "yes",
        "y",
        "ok",
        "예",
        "네",
        "승인",
    }:
        return ("accept", "accept")
    if normalized == "2":
        return ("acceptForSession", "acceptForSession")
    if normalized == "3" or lowered in {"decline", "reject", "no", "n", "아니요", "거절"}:
        return ("decline", "decline")
    if lowered in {"cancel", "skip", "dismiss", "건너뛰기", "취소"}:
        return ("cancel", "cancel")
    raise ApprovalReplyUnrecognizedError(
        "Unrecognized approval reply. Use 1 to approve, 2 to approve for this session, "
        + "3 to decline, or cancel to skip."
    )


def build_approval_decision_candidate_payloads(decision_payload: str) -> list[str]:
    return [decision_payload]


def _question_map(pending_request: JsonObject) -> dict[str, JsonObject]:
    raw_questions = pending_request.get("questions")
    if not isinstance(raw_questions, list) or not raw_questions:
        raise ReplyInputQuestionsMissingError("No pending input questions were available to answer.")

    questions: dict[str, JsonObject] = {}
    for index, raw_question in enumerate(raw_questions, start=1):
        if not isinstance(raw_question, dict):
            raise ReplyInputQuestionIdMissingError(f"Pending input question {index} did not include an id.")
        question_id = _clean_string(raw_question.get("id"))
        if not question_id:
            raise ReplyInputQuestionIdMissingError(f"Pending input question {index} did not include an id.")
        questions[question_id] = raw_question
    return questions


def _parse_multi_question_assignments(normalized: str) -> dict[str, str]:
    assignments: dict[str, str] = {}
    for raw_segment in normalized.split(";"):
        segment = raw_segment.strip()
        if not segment:
            continue
        if "=" not in segment:
            raise ReplyInputAssignmentFormatError("Multi-question replies must use question_id=value; other_id=value format.")
        question_id, raw_value = segment.split("=", 1)
        question_id = question_id.strip()
        if not question_id:
            raise ReplyInputAssignmentQuestionIdMissingError("A reply_input assignment was missing the question id.")
        assignments[question_id] = raw_value.strip()
    return assignments


def _require_matching_question_ids(
    questions: dict[str, JsonObject],
    assignments: dict[str, str],
) -> None:
    missing_question_ids = [
        question_id for question_id in questions if question_id not in assignments
    ]
    if missing_question_ids:
        raise ReplyInputMissingAnswersError("Missing answers for question ids: " + ", ".join(missing_question_ids))

    unknown_question_ids = [
        question_id for question_id in assignments if question_id not in questions
    ]
    if unknown_question_ids:
        raise ReplyInputUnknownQuestionError("Unknown question ids: " + ", ".join(unknown_question_ids))


def _option_labels(question: JsonObject) -> list[str]:
    options = question.get("options")
    if not isinstance(options, list):
        return []
    labels: list[str] = []
    for option in options:
        if not isinstance(option, dict):
            continue
        label = _clean_string(option.get("label"))
        if label:
            labels.append(label)
    return labels


def _clean_string(value: JsonValue | str | None) -> str:
    return value.strip() if isinstance(value, str) else ""
