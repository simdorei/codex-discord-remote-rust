from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from typing import Protocol

from codex_discord_slash_commands import SlashCommandBot, SlashInteraction

__all__ = [
    "QueueRetractResult",
    "RuntimeBridgeSessionRefresher",
    "RuntimeButtonQaRunner",
    "RuntimeDoctorMessageBuilder",
    "RuntimeQueueRetractor",
    "RuntimeSlashBridgeRunner",
    "RuntimeSlashChannel",
    "RuntimeSlashChunksSender",
    "RuntimeSlashCommandDeps",
    "RuntimeSlashInteraction",
    "RuntimeSlashResponseSender",
    "RuntimeSlashSourceMessage",
    "RuntimeSlashUser",
]


class RuntimeSlashUser(Protocol):
    id: int


class RuntimeSlashChannel(Protocol):
    id: int


class RuntimeSlashInteraction(SlashInteraction, Protocol):
    channel: RuntimeSlashChannel | None
    user: RuntimeSlashUser


@dataclass(frozen=True, slots=True)
class RuntimeSlashSourceMessage:
    author: RuntimeSlashUser
    channel: RuntimeSlashChannel


class RuntimeSlashChunksSender(Protocol):
    def __call__(
        self,
        interaction: RuntimeSlashInteraction,
        text: str,
        *,
        title: str,
    ) -> Awaitable[None]: ...


class RuntimeSlashBridgeRunner(Protocol):
    def __call__(
        self,
        interaction: RuntimeSlashInteraction,
        argv: list[str],
        title: str,
    ) -> Awaitable[tuple[int, str]]: ...


class RuntimeSlashResponseSender(Protocol):
    def __call__(
        self,
        interaction: RuntimeSlashInteraction,
        content: str,
        *,
        ephemeral: bool = False,
        context: str = "interaction_response",
    ) -> Awaitable[None]: ...


class RuntimeDoctorMessageBuilder(Protocol):
    def __call__(
        self,
        bot: SlashCommandBot,
        channel_id: int | None,
        channel: RuntimeSlashChannel | None,
    ) -> Awaitable[str]: ...


class RuntimeBridgeSessionRefresher(Protocol):
    def __call__(
        self,
        bot: SlashCommandBot,
        *,
        limit: int | None = None,
    ) -> Awaitable[str]: ...


QueueRetractResult = Mapping[str, str | int | bool | None]


class RuntimeQueueRetractor(Protocol):
    def __call__(
        self,
        *,
        channel_id: int | None,
        user_id: int | None,
        ref: str | None,
    ) -> Awaitable[tuple[str, QueueRetractResult]]: ...


class RuntimeButtonQaRunner(Protocol):
    def __call__(
        self,
        bot: SlashCommandBot,
        message: RuntimeSlashSourceMessage,
    ) -> Awaitable[str]: ...


@dataclass(frozen=True, slots=True)
class RuntimeSlashCommandDeps:
    check_allowed: Callable[[RuntimeSlashInteraction], bool]
    send_not_allowed: Callable[[RuntimeSlashInteraction], Awaitable[None]]
    send_chunks: RuntimeSlashChunksSender
    run_bridge: RuntimeSlashBridgeRunner
    build_doctor: RuntimeDoctorMessageBuilder
    build_runners: Callable[[], Awaitable[str]]
    retract_queued_ask: RuntimeQueueRetractor
    run_mirror_check: Callable[[], Awaitable[str]]
    refresh_bridge_session: RuntimeBridgeSessionRefresher
    qa_commands_enabled: Callable[[], bool]
    send_response: RuntimeSlashResponseSender
    run_button_qa: RuntimeButtonQaRunner
    log_line: Callable[[str], None]
