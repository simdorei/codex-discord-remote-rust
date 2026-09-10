from __future__ import annotations

from collections.abc import Callable, Iterable
from typing import Protocol

from codex_discord_components import format_approval_custom_id, format_busy_choice_custom_id


class ComponentViewChild(Protocol):
    label: str


ButtonPredicate = Callable[[ComponentViewChild], bool]

APPROVAL_LABEL_ANSWERS = {
    "Approve": "1",
    "Approve session": "2",
    "Reject": "3",
    "Cancel": "cancel",
}
BUSY_CHOICE_LABEL_ACTIONS = {
    "Steer now": "steer",
    "Queue next": "queue",
    "Stop reply": "stop",
    "Ignore": "ignore",
}


def assign_approval_button_custom_ids(
    children: Iterable[ComponentViewChild],
    target_thread_id: str,
    *,
    is_button: ButtonPredicate,
) -> None:
    for item in children:
        if not is_button(item):
            continue
        answer = APPROVAL_LABEL_ANSWERS.get(str(getattr(item, "label", "")))
        if answer:
            setattr(item, "custom_id", format_approval_custom_id(target_thread_id, answer))


def assign_busy_choice_button_custom_ids(
    children: Iterable[ComponentViewChild],
    choice_id: str | None,
    *,
    is_button: ButtonPredicate,
) -> None:
    if not choice_id:
        return
    for item in children:
        if not is_button(item):
            continue
        action = BUSY_CHOICE_LABEL_ACTIONS.get(str(getattr(item, "label", "")))
        if action:
            setattr(item, "custom_id", format_busy_choice_custom_id(choice_id, action))


def disable_busy_choice_steer_button(
    children: Iterable[ComponentViewChild],
    *,
    is_button: ButtonPredicate,
) -> None:
    for item in children:
        if is_button(item) and str(getattr(item, "label", "")) == "Steer now":
            setattr(item, "disabled", True)


def configure_busy_choice_buttons(
    children: Iterable[ComponentViewChild],
    choice_id: str | None,
    *,
    allow_steer: bool,
    is_button: ButtonPredicate,
) -> None:
    assign_busy_choice_button_custom_ids(children, choice_id, is_button=is_button)
    if not allow_steer:
        disable_busy_choice_steer_button(children, is_button=is_button)
