from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Literal, Protocol

from codex_discord_components import parse_busy_choice_custom_id
from codex_discord_text import format_log_text_len

PersistentBusyChoiceStatus = Literal[
    "unhandled",
    "missing",
    "denied",
    "steer_not_allowed",
    "ready",
]
BusyChoiceRecordValue = str | int | bool | float | None
BusyChoiceRecord = Mapping[str, BusyChoiceRecordValue]
BusyChoiceRecordGetter = Callable[[str], BusyChoiceRecord | None]
LogFunc = Callable[[str], None]
CHANNEL_UNAVAILABLE_MESSAGE = "Discord channel is unavailable. Send the message again to get fresh controls."


class PersistentBusyInteraction(Protocol): ...


class PersistentBusyChannel(Protocol): ...


class BusyComponentClearer(Protocol):
    def __call__(self, interaction: PersistentBusyInteraction, *, context: str) -> Awaitable[None]: ...


class BusyResponseSender(Protocol):
    def __call__(
        self,
        interaction: PersistentBusyInteraction,
        content: str,
        *,
        context: str,
    ) -> Awaitable[None]: ...


class BusyDirectFollowupSender(Protocol):
    def __call__(
        self,
        interaction: PersistentBusyInteraction,
        content: str,
        *,
        log_prefix: str,
        context: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class PersistentBusyChoiceResolution:
    status: PersistentBusyChoiceStatus
    choice_id: str = ""
    action: str = ""
    user_id: int = 0
    record: BusyChoiceRecord | None = None
    owner_user_id: int | None = None
    target_thread_id: str | None = None


@dataclass(frozen=True, slots=True)
class PersistentBusySourceAuthor:
    id: int


@dataclass(frozen=True, slots=True)
class PersistentBusySourceMessage:
    author: PersistentBusySourceAuthor
    channel: PersistentBusyChannel
    created_at: datetime


@dataclass(frozen=True, slots=True)
class PersistentBusyIgnoreDeps:
    clear_components: BusyComponentClearer
    send_response: BusyResponseSender
    log: LogFunc


@dataclass(frozen=True, slots=True)
class PersistentBusyChannelUnavailableDeps:
    send_followup: BusyDirectFollowupSender
    log: LogFunc


@dataclass(frozen=True, slots=True)
class PersistentBusyQueueFollowupDeps:
    send_followup: BusyDirectFollowupSender
    log: LogFunc


def normalize_record_thread_id(record: BusyChoiceRecord) -> str | None:
    return str(record["target_thread_id"] or "") or None


def normalize_record_user_id(value: BusyChoiceRecordValue) -> int:
    return int(str(value or "0"))


def normalize_record_channel_id(record: BusyChoiceRecord) -> int:
    return normalize_record_user_id(record["channel_id"])


def normalize_record_created_at(record: BusyChoiceRecord) -> datetime:
    value = record["created_at"]
    if not isinstance(value, int | float):
        raise TypeError("Persistent busy choice is missing its numeric creation time.")
    return datetime.fromtimestamp(float(value), tz=timezone.utc)


def make_persistent_busy_source_message(
    record: BusyChoiceRecord,
    channel: PersistentBusyChannel,
) -> PersistentBusySourceMessage:
    return PersistentBusySourceMessage(
        author=PersistentBusySourceAuthor(id=normalize_record_user_id(record["owner_user_id"])),
        channel=channel,
        created_at=normalize_record_created_at(record),
    )


def resolve_persistent_busy_choice(
    custom_id: str,
    *,
    user_id: int,
    get_busy_choice_record: BusyChoiceRecordGetter,
) -> PersistentBusyChoiceResolution:
    parsed = parse_busy_choice_custom_id(custom_id)
    if not parsed:
        return PersistentBusyChoiceResolution(status="unhandled", user_id=user_id)
    choice_id, action = parsed
    record = get_busy_choice_record(choice_id)
    if record is None:
        return PersistentBusyChoiceResolution(
            status="missing",
            choice_id=choice_id,
            action=action,
            user_id=user_id,
        )
    owner_user_id = normalize_record_user_id(record["owner_user_id"])
    target_thread_id = normalize_record_thread_id(record)
    if user_id != owner_user_id:
        return PersistentBusyChoiceResolution(
            status="denied",
            choice_id=choice_id,
            action=action,
            user_id=user_id,
            record=record,
            owner_user_id=owner_user_id,
            target_thread_id=target_thread_id,
        )
    if action == "steer" and not bool(record["allow_steer"]):
        return PersistentBusyChoiceResolution(
            status="steer_not_allowed",
            choice_id=choice_id,
            action=action,
            user_id=user_id,
            record=record,
            owner_user_id=owner_user_id,
            target_thread_id=target_thread_id,
        )
    return PersistentBusyChoiceResolution(
        status="ready",
        choice_id=choice_id,
        action=action,
        user_id=user_id,
        record=record,
        owner_user_id=owner_user_id,
        target_thread_id=target_thread_id,
    )


async def handle_persistent_busy_ignore(
    interaction: PersistentBusyInteraction,
    *,
    user_id: int,
    choice_id: str,
    target_thread_id: str | None,
    deps: PersistentBusyIgnoreDeps,
) -> bool:
    target = target_thread_id or "-"
    deps.log(f"busy_choice_persistent_ignore user={user_id} choice={choice_id} target={target}")
    await deps.clear_components(interaction, context="busy_choice_ignore")
    await deps.send_response(
        interaction,
        "Ignored.",
        context="busy_choice_persistent_ignore",
    )
    return True


async def handle_persistent_busy_channel_unavailable(
    interaction: PersistentBusyInteraction,
    *,
    action: str,
    choice_id: str,
    target_thread_id: str | None,
    deps: PersistentBusyChannelUnavailableDeps,
) -> bool:
    await deps.send_followup(
        interaction,
        CHANNEL_UNAVAILABLE_MESSAGE,
        log_prefix="button_followup",
        context="persistent_channel_unavailable",
    )
    target = target_thread_id or "-"
    deps.log(f"busy_choice_persistent_channel_unavailable action={action} choice={choice_id} target={target}")
    return True


async def handle_persistent_busy_queue_followup(
    interaction: PersistentBusyInteraction,
    *,
    user_id: int,
    choice_id: str,
    position: int,
    target_thread_id: str | None,
    prompt: str,
    deps: PersistentBusyQueueFollowupDeps,
) -> bool:
    await deps.send_followup(
        interaction,
        f"Queued at position {position}.",
        log_prefix="button_followup",
        context="persistent_queue_next",
    )
    target = target_thread_id or "-"
    prompt_len = format_log_text_len(prompt)
    log_message = f"busy_choice_persistent_queue user={user_id} choice={choice_id} position={position} target={target} prompt_len={prompt_len}"
    deps.log(log_message)
    return True
