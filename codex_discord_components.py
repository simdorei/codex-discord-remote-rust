"""Discord component custom-id helpers for the Codex adapter."""

from __future__ import annotations

import hashlib
import re
from collections.abc import Iterable
from typing import Protocol, runtime_checkable


BUSY_CHOICE_CUSTOM_ID_PREFIX = "codex_busy"
APPROVAL_CUSTOM_ID_PREFIX = "codex_approval"
INPUT_CHOICE_CUSTOM_ID_PREFIX = "codex_input"

DiscordIdValue = int | str | bytes | bytearray


class ComponentChildLike(Protocol):
    @property
    def custom_id(self) -> str | None: ...


@runtime_checkable
class ComponentChildrenContainer(Protocol):
    @property
    def children(self) -> Iterable[ComponentChildLike] | None: ...


@runtime_checkable
class ComponentComponentsContainer(Protocol):
    @property
    def components(self) -> Iterable[ComponentChildLike] | None: ...


ComponentRowLike = ComponentChildrenContainer | ComponentComponentsContainer


class ComponentMessageLike(Protocol):
    @property
    def components(self) -> Iterable[ComponentRowLike] | None: ...


class PersistentComponentMessageLike(Protocol):
    @property
    def id(self) -> DiscordIdValue: ...


class PersistentComponentInteractionLike(Protocol):
    @property
    def message(self) -> PersistentComponentMessageLike | None: ...


def parse_busy_choice_custom_id(custom_id: str) -> tuple[str, str] | None:
    parts = str(custom_id or "").split(":")
    if len(parts) != 3 or parts[0] != BUSY_CHOICE_CUSTOM_ID_PREFIX:
        return None
    choice_id = parts[1].strip()
    action = parts[2].strip()
    if not re.fullmatch(r"[0-9a-f]{24}", choice_id):
        return None
    if action not in {"steer", "queue", "stop", "ignore"}:
        return None
    return choice_id, action


def format_busy_choice_custom_id(choice_id: str, action: str) -> str:
    return f"{BUSY_CHOICE_CUSTOM_ID_PREFIX}:{choice_id}:{action}"


def get_component_children(component: ComponentRowLike) -> list[ComponentChildLike]:
    if isinstance(component, ComponentChildrenContainer) and component.children is not None:
        try:
            return list(component.children)
        except TypeError:
            return []
    if isinstance(component, ComponentComponentsContainer) and component.components is not None:
        try:
            return list(component.components)
        except TypeError:
            return []
    return []


def get_busy_choice_custom_ids_from_message(message: ComponentMessageLike) -> list[str]:
    custom_ids: list[str] = []
    for row in message.components or ():
        for child in get_component_children(row):
            custom_id = child.custom_id
            if parse_busy_choice_custom_id(str(custom_id or "")):
                custom_ids.append(str(custom_id))
    return custom_ids


def format_approval_custom_id(target_thread_id: str, answer: str) -> str:
    return f"{APPROVAL_CUSTOM_ID_PREFIX}:{target_thread_id}:{answer}"


def parse_approval_custom_id(custom_id: str) -> tuple[str, str] | None:
    parts = str(custom_id or "").split(":", 2)
    if len(parts) != 3 or parts[0] != APPROVAL_CUSTOM_ID_PREFIX:
        return None
    target_thread_id = parts[1].strip()
    answer = parts[2].strip()
    if not target_thread_id or answer not in {"1", "2", "3", "cancel"}:
        return None
    return target_thread_id, answer


def is_safe_persistent_input_value(value: str) -> bool:
    return bool(re.fullmatch(r"[A-Za-z0-9_.-]{1,20}", str(value or "")))


def format_input_choice_custom_id(target_thread_id: str, value: str) -> str | None:
    normalized_value = str(value or "").strip()
    if not is_safe_persistent_input_value(normalized_value):
        return None
    custom_id = f"{INPUT_CHOICE_CUSTOM_ID_PREFIX}:{target_thread_id}:{normalized_value}"
    if len(custom_id) > 100:
        return None
    return custom_id


def parse_input_choice_custom_id(custom_id: str) -> tuple[str, str] | None:
    parts = str(custom_id or "").split(":", 2)
    if len(parts) != 3 or parts[0] != INPUT_CHOICE_CUSTOM_ID_PREFIX:
        return None
    target_thread_id = parts[1].strip()
    value = parts[2].strip()
    if not target_thread_id or not is_safe_persistent_input_value(value):
        return None
    return target_thread_id, value


def get_persistent_component_claim_key(
    interaction: PersistentComponentInteractionLike,
    custom_id: str,
) -> str | None:
    parsed_approval = parse_approval_custom_id(custom_id)
    parsed_input = parse_input_choice_custom_id(custom_id)
    if parsed_approval:
        kind = APPROVAL_CUSTOM_ID_PREFIX
    elif parsed_input:
        kind = INPUT_CHOICE_CUSTOM_ID_PREFIX
    else:
        return None
    message = interaction.message
    if message is None:
        return None
    raw_key = f"{kind}:{int(message.id)}"
    return hashlib.sha256(raw_key.encode("utf-8")).hexdigest()
