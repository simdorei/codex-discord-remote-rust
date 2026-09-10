from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, cast, TypeAlias

import codex_discord_bot_skill_slash_runtime as discord_bot_skill_slash_runtime
import codex_discord_delivery as discord_delivery
import codex_discord_slash_ask_flow as discord_slash_ask_flow
import codex_discord_slash_prompt_commands as discord_slash_prompt_commands
from codex_discord_bot_shapes import (
    BusyChoiceAuthor,
    DiscordMessageableChannel,
    SkillSlashSourceAuthor,
    SlashAskSourceMessage,
)
ModuleValue: TypeAlias = object


class InteractionCommandLike(Protocol):
    @property
    def name(self) -> str | None: ...


@dataclass(frozen=True, slots=True)
class BotSkillSlashAdapterRuntime:
    module: ModuleType

    async def handle_skill_slash_plain_ask(
        self,
        source_message: SlashAskSourceMessage,
        prompt: str,
        *,
        target_thread_id: str | None,
    ) -> None:
        handle_plain_ask = cast(Callable[..., Awaitable[None]], getattr(self.module, "handle_plain_ask"))
        await handle_plain_ask(source_message, prompt, target_thread_id=target_thread_id)

    def get_skill_slash_interaction_command_name(self, interaction: ModuleValue) -> str:
        command = cast(InteractionCommandLike | None, getattr(interaction, "command", None))
        return "-" if command is None else str(command.name or "-")

    def make_skill_slash_source_message(
        self,
        channel: discord_slash_prompt_commands.PromptChannel,
        user: discord_slash_prompt_commands.PromptUser,
    ) -> SlashAskSourceMessage:
        return SlashAskSourceMessage(
            channel=self._resolve_skill_slash_channel(channel),
            author=SkillSlashSourceAuthor(user),
        )

    async def send_skill_slash_interaction_chunks(
        self,
        interaction: ModuleValue,
        text: str,
        *,
        title: str,
    ) -> None:
        require_interaction = cast(
            Callable[[object], discord_slash_prompt_commands.SkillSlashInteraction],
            getattr(self.module, "require_discord_interaction"),
        )
        send_interaction_chunks = cast(Callable[..., Awaitable[None]], getattr(self.module, "send_interaction_chunks"))
        await send_interaction_chunks(require_interaction(interaction), text, title=title)

    def make_skill_slash_runtime(self) -> discord_bot_skill_slash_runtime.BotSkillSlashRuntime[SlashAskSourceMessage]:
        return discord_bot_skill_slash_runtime.BotSkillSlashRuntime(
            discord_bot_skill_slash_runtime.BotSkillSlashRuntimeDeps(
                prompt_deps=discord_slash_prompt_commands.SkillSlashPromptDeps(
                    send_interaction_chunks=self.send_skill_slash_interaction_chunks,
                    send_direct_followup=cast(
                        discord_slash_prompt_commands.DirectFollowupSender,
                        getattr(self.module, "send_skill_slash_direct_followup"),
                    ),
                    handle_plain_ask=self.handle_skill_slash_plain_ask,
                    get_mirrored_codex_thread_id=lambda channel_id: cast(
                        Callable[[int | None], str | None],
                        getattr(self.module, "get_mirrored_codex_thread_id"),
                    )(channel_id),
                    describe_mirrored_project_channel=lambda channel_id: cast(
                        Callable[[int | None], str],
                        getattr(self.module, "describe_mirrored_project_channel"),
                    )(channel_id),
                    get_interaction_command_name=self.get_skill_slash_interaction_command_name,
                    format_log_text_len=lambda text: cast(
                        Callable[[str], str],
                        getattr(self.module, "format_log_text_len_as_text"),
                    )(text),
                    make_source_message=self.make_skill_slash_source_message,
                    log_line=lambda message: cast(Callable[[str], None], getattr(self.module, "log_line"))(message),
                )
            )
        )

    def make_slash_ask_source_message(
        self,
        channel: discord_slash_prompt_commands.PromptChannel,
        user: discord_slash_prompt_commands.PromptUser,
    ) -> SlashAskSourceMessage:
        return SlashAskSourceMessage(
            channel=self._resolve_skill_slash_channel(channel),
            author=cast(BusyChoiceAuthor, user),
        )

    def _resolve_skill_slash_channel(
        self,
        channel: discord_slash_prompt_commands.PromptChannel,
        /,
    ) -> DiscordMessageableChannel:
        require_messageable = cast(
            Callable[[object], object],
            getattr(self.module, "require_discord_messageable"),
        )
        resolved_channel = require_messageable(channel)
        if not isinstance(getattr(resolved_channel, "id", None), int):
            raise TypeError("Skill-slash channel must have an integer id.")
        if not callable(getattr(resolved_channel, "send", None)):
            raise TypeError("Skill-slash channel must provide send().")
        return cast(DiscordMessageableChannel, resolved_channel)

    def is_slash_ask_messageable_channel(self, channel: discord_slash_prompt_commands.PromptChannel) -> bool:
        return hasattr(channel, "send")

    async def handle_slash_ask(
        self,
        interaction: discord_slash_ask_flow.SlashAskInteraction[
            discord_slash_prompt_commands.PromptChannel,
            discord_slash_prompt_commands.PromptUser,
        ],
        prompt: str,
    ) -> None:
        await discord_slash_ask_flow.handle_slash_ask(
            interaction,
            prompt,
            deps=discord_slash_ask_flow.SlashAskFlowDeps(
                send_interaction_chunks=cast(
                    discord_slash_ask_flow.SlashAskChunkSender[
                        discord_slash_prompt_commands.PromptChannel,
                        discord_slash_prompt_commands.PromptUser,
                    ],
                    getattr(self.module, "send_interaction_chunks"),
                ),
                send_direct_followup=cast(
                    discord_slash_ask_flow.SlashAskFollowupSender[
                        discord_slash_prompt_commands.PromptChannel,
                        discord_slash_prompt_commands.PromptUser,
                    ],
                    getattr(self.module, "send_direct_followup"),
                ),
                handle_plain_ask=cast(
                    discord_slash_ask_flow.SlashAskHandler[SlashAskSourceMessage],
                    getattr(self.module, "handle_plain_ask"),
                ),
                get_mirrored_thread_id=cast(
                    Callable[[int | None], str | None],
                    getattr(self.module, "get_mirrored_codex_thread_id"),
                ),
                describe_project_channel=cast(
                    Callable[[int | None], str],
                    getattr(self.module, "describe_mirrored_project_channel"),
                ),
                get_command_name=cast(
                    Callable[
                        [
                            discord_slash_ask_flow.SlashAskInteraction[
                                discord_slash_prompt_commands.PromptChannel,
                                discord_slash_prompt_commands.PromptUser,
                            ]
                        ],
                        str,
                    ],
                    discord_delivery.get_interaction_command_name,
                ),
                format_text_len=cast(Callable[[str], int], getattr(self.module, "format_log_text_len")),
                is_messageable_channel=self.is_slash_ask_messageable_channel,
                make_source_message=self.make_slash_ask_source_message,
                log=cast(Callable[[str], None], getattr(self.module, "log_line")),
            ),
        )

    async def handle_slash_interview(
        self,
        interaction: discord_slash_prompt_commands.SkillSlashInteraction,
        prompt: str,
    ) -> None:
        await cast(
            discord_bot_skill_slash_runtime.BotSkillSlashRuntime[SlashAskSourceMessage],
            getattr(self.module, "SKILL_SLASH_RUNTIME"),
        ).handle_interview(interaction, prompt)
