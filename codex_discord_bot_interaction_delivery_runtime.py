from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from contextlib import AbstractContextManager, nullcontext
from dataclasses import dataclass
from types import ModuleType
from typing import cast

import codex_discord_bridge_send as discord_bridge_send
import codex_discord_delivery as discord_delivery
import codex_discord_delivery_interactions as discord_delivery_interactions
import codex_discord_delivery_state as discord_delivery_state
import codex_discord_session_mirror_archive as discord_session_mirror_archive
import codex_discord_interrupt_context as discord_interrupt_context


@dataclass(frozen=True, slots=True)
class BotInteractionDeliveryRuntime:
    module: ModuleType

    async def run_bridge_and_send(
        self,
        target: discord_delivery_state.Messageable,
        argv: list[str],
        title: str,
        failure_title: str | None = None,
        archive_cleanup_owner: discord_session_mirror_archive.ArchiveMirrorCleanupOwner | None = None,
    ) -> tuple[int, str]:
        with self._stop_scope(argv):
            return await discord_bridge_send.run_bridge_and_send(
                target,
                argv,
                title,
                failure_title=failure_title,
                archive_cleanup_owner=archive_cleanup_owner,
                run_bridge_command_func=cast(Callable[[list[str]], tuple[int, str]], getattr(self.module, "run_bridge_command")),
                cleanup_archive_mirror_after_bridge_command_func=cast(
                    Callable[
                        [discord_session_mirror_archive.ArchiveMirrorCleanupOwner | None, list[str], int, str],
                        Awaitable[str | None],
                    ],
                    getattr(self.module, "cleanup_archive_mirror_after_bridge_command"),
                ),
                split_delivery_chunks_func=cast(Callable[[str], list[str]], getattr(self.module, "split_delivery_chunks")),
                send_chunks_func=cast(discord_bridge_send.SendChunksFunc[discord_delivery_state.Messageable, int], getattr(self.module, "send_chunks")),
                format_log_argv_func=cast(Callable[[list[str]], str], getattr(self.module, "format_log_argv")),
                log_func=cast(discord_delivery_state.LogFunc, getattr(self.module, "log_line")),
            )

    async def run_interaction_bridge_and_send(
        self,
        interaction: discord_delivery_interactions.FollowupInteraction,
        argv: list[str],
        title: str,
        failure_title: str | None = None,
    ) -> tuple[int, str]:
        run_bridge_command = cast(Callable[[list[str]], tuple[int, str]], getattr(self.module, "run_bridge_command"))
        with self._stop_scope(argv):
            exit_code, output = await asyncio.to_thread(run_bridge_command, argv)
        prefix = title if exit_code == 0 else f"{failure_title or title} failed (exit {exit_code})"
        log = cast(discord_delivery_state.LogFunc, getattr(self.module, "log_line"))
        format_log_argv = cast(Callable[[list[str]], str], getattr(self.module, "format_log_argv"))
        log(
            f"slash_bridge_done command={discord_delivery.get_interaction_command_name(interaction)} "
            + f"title={title!r} exit={exit_code} argv={format_log_argv(argv)}"
        )
        await self.send_interaction_chunks(
            interaction,
            f"{prefix}\n\n{output or '(no output)'}",
            title=title,
            exit_code=exit_code,
        )
        return exit_code, output

    def _stop_scope(self, argv: list[str]) -> AbstractContextManager[None]:
        if argv and argv[0] == "stop":
            return discord_interrupt_context.discord_remote_stop_scope()
        return nullcontext()

    async def send_interaction_chunks(
        self,
        interaction: discord_delivery_interactions.FollowupInteraction,
        text: str,
        *,
        title: str,
        exit_code: int | None = None,
    ) -> None:
        await self.send_followup_chunks(
            interaction,
            text,
            title=title,
            exit_code=exit_code,
            log_prefix="slash_response",
        )

    async def send_followup_chunks(
        self,
        interaction: discord_delivery_interactions.FollowupInteraction,
        text: str,
        *,
        title: str,
        exit_code: int | None = None,
        log_prefix: str = "followup_response",
        ephemeral: bool = False,
        allow_during_stop: bool = False,
    ) -> None:
        await discord_delivery.send_followup_chunks(
            self._delivery_state(),
            interaction,
            text,
            log_func=cast(discord_delivery_state.LogFunc, getattr(self.module, "log_line")),
            title=title,
            exit_code=exit_code,
            log_prefix=log_prefix,
            ephemeral=ephemeral,
            allow_during_stop=allow_during_stop,
        )

    async def send_busy_followup_chunks(
        self,
        interaction: discord_delivery_interactions.FollowupInteraction,
        content: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
        ephemeral: bool,
    ) -> None:
        await self.send_followup_chunks(
            interaction,
            content,
            title=title,
            exit_code=exit_code,
            log_prefix=log_prefix,
            ephemeral=ephemeral,
        )

    async def send_direct_followup(
        self,
        interaction: discord_delivery_interactions.FollowupInteraction,
        content: str,
        *,
        ephemeral: bool = False,
        view: discord_delivery_interactions.FollowupView | None = None,
        log_prefix: str = "direct_followup",
        context: str = "",
        allow_during_stop: bool = False,
    ) -> None:
        await discord_delivery.send_direct_followup(
            self._delivery_state(),
            interaction,
            content,
            log_func=cast(discord_delivery_state.LogFunc, getattr(self.module, "log_line")),
            ephemeral=ephemeral,
            view=view,
            log_prefix=log_prefix,
            context=context,
            allow_during_stop=allow_during_stop,
        )

    async def send_skill_slash_direct_followup(
        self,
        interaction: discord_delivery_interactions.FollowupInteraction,
        text: str,
        *,
        ephemeral: bool,
        log_prefix: str,
        context: str,
    ) -> None:
        await self.send_direct_followup(
            interaction,
            text,
            ephemeral=ephemeral,
            log_prefix=log_prefix,
            context=context,
        )

    def _delivery_state(self) -> discord_delivery_state.DiscordDeliveryState:
        return cast(discord_delivery_state.DiscordDeliveryState, getattr(self.module, "DISCORD_DELIVERY_STATE"))
