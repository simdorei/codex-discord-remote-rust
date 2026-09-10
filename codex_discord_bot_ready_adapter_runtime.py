from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, cast, TypeAlias

import codex_discord_plain_ask as discord_plain_ask
import codex_discord_ready_runtime as discord_ready_runtime
import codex_discord_store as discord_store
ModuleValue: TypeAlias = object


class DiscordAbcModule(Protocol):
    Messageable: type[ModuleValue]


class DiscordModule(Protocol):
    abc: DiscordAbcModule


@dataclass(frozen=True, slots=True)
class BotReadyAdapterRuntime:
    module: ModuleType

    def make_ready_runtime(self) -> discord_ready_runtime.ReadyRuntime[ModuleValue]:
        return discord_ready_runtime.ReadyRuntime(
            delivery_exceptions=self.delivery_exceptions(),
            is_messageable=self.is_messageable,
            get_startup_probe_targets=self.get_startup_probe_targets,
            get_startup_probe_timeout=self.get_startup_probe_timeout,
            format_traceback=self.format_traceback,
            build_prompt_with_discord_attachments=self.build_prompt_with_discord_attachments,
            send_chunks=self.send_chunks,
            cleanup_expired_busy_choices=self.cleanup_expired_busy_choices,
            cleanup_expired_persistent_component_claims=self.cleanup_expired_persistent_component_claims,
            cleanup_processed_messages=self.cleanup_processed_messages,
            cleanup_session_mirror_events=self.cleanup_session_mirror_events,
            cleanup_stale_busy_choice_components_in_channel=self.cleanup_stale_busy_choice_components_in_channel,
            log=self.log_line,
        )

    def is_messageable(self, channel: ModuleValue) -> bool:
        return isinstance(channel, self.discord_module().abc.Messageable)

    def get_startup_probe_targets(
        self,
        allowed_channel_ids: set[int],
        startup_channel_id: int | None,
    ) -> list[tuple[str, int]]:
        return cast(
            Callable[[set[int], int | None], list[tuple[str, int]]],
            self._module_func("get_startup_probe_targets"),
        )(allowed_channel_ids, startup_channel_id)

    def get_startup_probe_timeout(self) -> float:
        return cast(Callable[[], float], self._module_func("get_startup_channel_probe_timeout"))()

    def format_traceback(self) -> str:
        traceback_module = cast(ModuleType, getattr(self.module, "traceback"))
        return cast(Callable[[], str], getattr(traceback_module, "format_exc"))()

    async def build_prompt_with_discord_attachments(
        self,
        message: discord_plain_ask.PlainAskPreparedMessage,
        prompt: str,
    ) -> str:
        return await cast(
            discord_plain_ask.BuildPromptWithAttachmentsFunc[discord_plain_ask.PlainAskPreparedMessage],
            self._module_func("build_prompt_with_discord_attachments"),
        )(message, prompt)

    async def send_chunks(
        self,
        channel: discord_plain_ask.PlainAskChannel,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> int:
        result = await cast(discord_plain_ask.SendChunksFunc[int | None], self._module_func("send_chunks"))(
            channel,
            text,
            context=context,
        )
        return 0 if result is None else result

    def cleanup_expired_busy_choices(self) -> int:
        return cast(Callable[[], int], self._module_func("cleanup_expired_busy_choices"))()

    def cleanup_expired_persistent_component_claims(self) -> int:
        return cast(Callable[[], int], self._module_func("cleanup_expired_persistent_component_claims"))()

    def cleanup_processed_messages(self) -> int:
        return discord_store.cleanup_processed_discord_messages(
            self.mirror_db_path(),
            retention_seconds=cast(float, getattr(self.module, "PROCESSED_MESSAGE_RETENTION_SECONDS")),
        )

    def cleanup_session_mirror_events(self) -> int:
        return discord_store.cleanup_session_mirror_events(
            self.mirror_db_path(),
            retention_seconds=cast(float, getattr(self.module, "SESSION_MIRROR_EVENT_RETENTION_SECONDS")),
        )

    async def cleanup_stale_busy_choice_components_in_channel(self, channel: ModuleValue) -> int:
        return await cast(
            Callable[[object], Awaitable[int]],
            self._module_func("cleanup_stale_busy_choice_components_in_channel"),
        )(channel)

    def delivery_exceptions(self) -> tuple[type[BaseException], ...]:
        return cast(tuple[type[BaseException], ...], getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS"))

    def mirror_db_path(self) -> Path:
        return cast(Path, getattr(self.module, "MIRROR_DB_PATH"))

    def discord_module(self) -> DiscordModule:
        return cast(DiscordModule, getattr(self.module, "discord"))

    def log_line(self, message: str) -> None:
        cast(Callable[[str], None], self._module_func("log_line"))(message)

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
