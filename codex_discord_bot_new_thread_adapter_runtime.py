from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, cast, TypeAlias

import codex_discord_bridge_process as bridge_process
import codex_discord_delivery_interactions as discord_delivery_interactions
import codex_discord_new_thread_flow as discord_new_thread_flow
import codex_discord_prefix_new_command as discord_prefix_new_command
from codex_discord_text import format_log_text_len
ModuleValue: TypeAlias = object


class SlashNewUser(Protocol):
    @property
    def id(self) -> int: ...


class SlashNewInteraction(discord_delivery_interactions.FollowupInteraction, Protocol):
    @property
    def user(self) -> SlashNewUser: ...


class NewThreadLike(discord_new_thread_flow.DiscordThreadLike, Protocol):
    pass


class BridgeMirrorStatus(Protocol):
    def choose_thread(self, thread_id: str, fallback: str | None = None) -> ModuleValue: ...


class SendInteractionChunks(Protocol):
    def __call__(
        self,
        interaction: discord_delivery_interactions.FollowupInteraction,
        text: str,
        *,
        title: str,
        exit_code: int | None = None,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class BotNewThreadAdapterRuntime:
    module: ModuleType

    async def run_discord_new_thread(
        self,
        bot: ModuleValue,
        discord_channel_id: int | None,
        prompt: str,
    ) -> tuple[int, str]:
        return await discord_new_thread_flow.run_discord_new_thread(
            bot,
            discord_channel_id,
            prompt,
            deps=discord_new_thread_flow.NewThreadFlowDeps(
                resolve_new_thread_cwd=cast(
                    Callable[[int | None], str | None],
                    getattr(self.module, "resolve_discord_new_thread_cwd"),
                ),
                run_bridge_command=cast(Callable[[list[str]], tuple[int, str]], getattr(self.module, "run_bridge_command")),
                parse_bridge_output_value=bridge_process.parse_bridge_output_value,
                choose_thread=cast(BridgeMirrorStatus, getattr(self.module, "BRIDGE_MIRROR_STATUS")).choose_thread,
                get_project_key=cast(Callable[[object], str], getattr(self.module, "get_project_key")),
                resolve_project_channel_id=cast(
                    Callable[[int | None, str], int | None],
                    getattr(self.module, "resolve_discord_new_thread_project_channel_id"),
                ),
                mirror_single_codex_thread=cast(
                    discord_new_thread_flow.MirrorSingleCodexThreadFunc[object, NewThreadLike],
                    getattr(self.module, "mirror_single_codex_thread"),
                ),
                prepare_mapped_session_mirror_output=cast(
                    Callable[[NewThreadLike, str], Awaitable[bool]],
                    getattr(self.module, "prepare_mapped_session_mirror_output"),
                ),
                delivery_exceptions=cast(tuple[type[BaseException], ...], getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS")),
                log=cast(Callable[[str], None], getattr(self.module, "log_line")),
            ),
        )

    async def handle_slash_new(
        self,
        bot: ModuleValue,
        interaction: SlashNewInteraction,
        prompt: str,
    ) -> None:
        log = cast(Callable[[str], None], getattr(self.module, "log_line"))
        log(
            f"slash_new_dispatch channel={interaction.channel_id} "
            + f"user={interaction.user.id} prompt_len={format_log_text_len(prompt)}"
        )
        channel_id = cast(int | None, interaction.channel_id)
        run_discord_new_thread = cast(
            Callable[[object, int | None, str], Awaitable[tuple[int, str]]],
            getattr(self.module, "run_discord_new_thread"),
        )
        exit_code, output = await run_discord_new_thread(bot, channel_id, prompt)
        log(f"slash_new_done channel={interaction.channel_id} exit={exit_code}")
        send_interaction_chunks = cast(SendInteractionChunks, getattr(self.module, "send_interaction_chunks"))
        await send_interaction_chunks(interaction, output, title="New", exit_code=exit_code)

    async def run_prefix_discord_new_thread(
        self,
        bot: discord_prefix_new_command.BotLike,
        channel_id: int | None,
        prompt: str,
    ) -> tuple[int, str]:
        return await self.run_discord_new_thread(bot, channel_id, prompt)
