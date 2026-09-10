from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import codex_discord_bot_session_mirror_factory as discord_bot_session_mirror_factory
import codex_discord_bot_session_mirror_runtime as discord_bot_session_mirror_runtime
import codex_discord_session_mirror_item_delivery as discord_session_mirror_item_delivery
import codex_discord_session_mirror_target as discord_session_mirror_target
import codex_discord_store as discord_store
from codex_session_events import JsonEvent
ModuleValue: TypeAlias = object


MessageableChannel: TypeAlias = object


class DiscordAbcModule(Protocol):
    Messageable: type[MessageableChannel]


class DiscordModule(Protocol):
    DiscordException: type[Exception]
    abc: DiscordAbcModule


@dataclass(frozen=True, slots=True)
class BotSessionMirrorAdapterRuntime:
    module: ModuleType

    def make_session_mirror_runtime(
        self,
    ) -> discord_bot_session_mirror_runtime.SessionMirrorRuntime[MessageableChannel]:
        discord_module = self._discord_module()
        return discord_bot_session_mirror_factory.make_session_mirror_runtime(
            target_limit=cast(int, getattr(self.module, "SESSION_MIRROR_TARGET_LIMIT")),
            archive_backlog_max_events_default=cast(
                int,
                getattr(self.module, "SESSION_MIRROR_ARCHIVE_BACKLOG_MAX_EVENTS_DEFAULT"),
            ),
            delivery_exceptions=cast(
                tuple[type[BaseException], ...],
                getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS"),
            ),
            fetch_failure_types=(discord_module.DiscordException,),
            get_db_path=lambda: cast(Path, getattr(self.module, "MIRROR_DB_PATH")),
            load_targets_in_thread=self.load_targets_in_thread,
            create_task=lambda coro: asyncio.create_task(coro),
            sleep=lambda seconds: asyncio.sleep(seconds),
            is_messageable=self.is_messageable,
            parse_interactive_notice=cast(
                discord_session_mirror_item_delivery.ParseInteractiveNotice,
                self._module_func("parse_interactive_notice"),
            ),
            send_interactive_prompt=cast(
                discord_session_mirror_item_delivery.SessionMirrorInteractiveSender[
                    MessageableChannel
                ],
                self._module_func("send_interactive_prompt"),
            ),
            send_chunks=self.send_chunks,
            send_attachment=self.send_attachment,
            collect_session_mirror_items=cast(
                discord_session_mirror_target.SessionMirrorItemCollector[JsonEvent],
                self._module_func("collect_session_mirror_items"),
            ),
            get_archive_skip_logged=self.get_archive_skip_logged,
            resolve_target_ref=self.resolve_target_ref,
            is_active_output_target=self.is_active_output_target,
            is_pending_cursor_target=self.is_pending_cursor_target,
            clear_pending_cursor_target=self.clear_pending_cursor_target,
            update_session_mirror_cursor=self.update_session_mirror_cursor,
            get_or_init_session_mirror_cursor=self.get_or_init_session_mirror_cursor,
            has_session_mirror_event=self.has_session_mirror_event,
            claim_session_mirror_event=self.claim_session_mirror_event,
            release_session_mirror_output_target=cast(
                Callable[[str], Awaitable[bool]],
                self._module_func("release_session_mirror_output_target"),
            ),
            send_typing_pulse=self.send_typing_pulse,
            events_bridge=cast(
                discord_bot_session_mirror_factory.SessionMirrorEventsBridge,
                getattr(self.module, "BRIDGE_SESSION_MIRROR_EVENTS"),
            ),
            log=cast(Callable[[str], None], self._module_func("log_line")),
        )

    async def load_targets_in_thread(
        self,
        db_path: Path,
        limit: int,
    ) -> Sequence[discord_bot_session_mirror_runtime.SessionMirrorTargetMapping]:
        return await asyncio.to_thread(
            discord_store.get_session_mirror_targets,
            db_path,
            limit=limit,
        )

    async def send_chunks(
        self,
        channel: MessageableChannel,
        content: str,
        *,
        context: str,
    ) -> None:
        await cast(
            discord_session_mirror_item_delivery.SessionMirrorChunkSender[MessageableChannel],
            self._module_func("send_prompt_chunks"),
        )(
            channel,
            content,
            context=context,
        )

    async def send_attachment(
        self,
        channel: MessageableChannel,
        content: str,
        attachment_url: str,
        filename: str,
        *,
        context: str,
    ) -> None:
        await cast(
            discord_session_mirror_item_delivery.SessionMirrorAttachmentSender[MessageableChannel],
            self._module_func("send_session_mirror_attachment"),
        )(
            channel,
            content,
            attachment_url,
            filename,
            context=context,
        )

    async def send_typing_pulse(self, channel: MessageableChannel, target_thread_id: str, context: str) -> None:
        starter = cast(Callable[..., None], self._module_func("start_session_mirror_typing_pulse"))
        release = cast(
            Callable[[str], Awaitable[bool]],
            self._module_func("release_session_mirror_output_target"),
        )
        starter(
            channel,
            target_thread_id,
            context,
            channel_typing=self._module_func("channel_typing"),
            log=cast(Callable[[str], None], self._module_func("log_line")),
            on_start_error=release,
        )

    def get_archive_skip_logged(
        self,
        owner: discord_bot_session_mirror_runtime.SessionMirrorOwner[MessageableChannel],
    ) -> set[str]:
        return cast(set[str], getattr(owner, "_session_mirror_archive_skip_logged"))

    def is_messageable(self, channel: MessageableChannel) -> bool:
        return isinstance(channel, self._discord_module().abc.Messageable)

    def resolve_target_ref(self, target_thread_id: str) -> tuple[str | None, str]:
        return cast(
            Callable[[str], tuple[str | None, str]],
            self._module_func("resolve_target_ref"),
        )(target_thread_id)

    def is_active_output_target(self, target_thread_id: str) -> bool:
        return cast(Callable[[str], bool], self._module_func("is_active_session_mirror_output_target"))(
            target_thread_id,
        )

    def is_pending_cursor_target(self, target_thread_id: str) -> bool:
        return cast(Callable[[str], bool], self._module_func("is_pending_session_mirror_cursor_target"))(
            target_thread_id,
        )

    def clear_pending_cursor_target(self, target_thread_id: str) -> None:
        cast(Callable[[str], None], self._module_func("clear_pending_session_mirror_cursor_target"))(
            target_thread_id,
        )

    def update_session_mirror_cursor(self, codex_thread_id: str, rollout_path: str, cursor: int) -> None:
        cast(Callable[[str, str, int], None], self._module_func("update_session_mirror_cursor"))(
            codex_thread_id,
            rollout_path,
            cursor,
        )

    def get_or_init_session_mirror_cursor(
        self,
        codex_thread_id: str,
        rollout_path: str,
        initial_cursor: int,
    ) -> int:
        return cast(
            Callable[[str, str, int], int],
            self._module_func("get_or_init_session_mirror_cursor"),
        )(
            codex_thread_id,
            rollout_path,
            initial_cursor,
        )

    def has_session_mirror_event(self, event_digest: str, codex_thread_id: str) -> bool:
        return cast(Callable[[str, str], bool], self._module_func("has_session_mirror_event"))(
            event_digest,
            codex_thread_id,
        )

    def claim_session_mirror_event(self, event_digest: str, codex_thread_id: str) -> bool:
        return cast(Callable[[str, str], bool], self._module_func("claim_session_mirror_event"))(
            event_digest,
            codex_thread_id,
        )

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))

    def _discord_module(self) -> DiscordModule:
        return cast(DiscordModule, getattr(self.module, "discord"))
