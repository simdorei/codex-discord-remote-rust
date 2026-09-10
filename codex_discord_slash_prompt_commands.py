from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import TypeAlias

from codex_discord_slash_commands import SlashCommandBot, SlashInteraction
from codex_discord_slash_skill_prompts import (
    DirectFollowupSender,
    InteractionChunksSender,
    PlainAskHandler,
    PromptChannel,
    PromptUser,
    SkillSlashInteraction,
    SkillSlashPromptDeps,
    SkillSlashPromptSpec,
    handle_skill_slash_prompt,
)

__all__ = [
    "DirectFollowupSender",
    "InteractionChunksSender",
    "PlainAskHandler",
    "PromptChannel",
    "PromptSlashCommandDeps",
    "PromptSlashHandler",
    "PromptSlashInteraction",
    "PromptUser",
    "SkillSlashInteraction",
    "SkillSlashPromptDeps",
    "SkillSlashPromptSpec",
    "handle_skill_slash_prompt",
    "register_prompt_slash_commands",
]

PromptSlashInteraction: TypeAlias = SlashInteraction
PromptSlashHandler: TypeAlias = Callable[[PromptSlashInteraction, str], Awaitable[None]]


@dataclass(frozen=True, slots=True)
class PromptSlashCommandDeps:
    check_allowed: Callable[[PromptSlashInteraction], bool]
    send_not_allowed: Callable[[PromptSlashInteraction], Awaitable[None]]
    handle_new: PromptSlashHandler
    handle_ask: PromptSlashHandler
    handle_interview: PromptSlashHandler


def register_prompt_slash_commands(bot: SlashCommandBot, deps: PromptSlashCommandDeps) -> None:
    async def run_prompt_command(
        interaction: PromptSlashInteraction,
        prompt: str,
        handler: PromptSlashHandler,
    ) -> None:
        if not deps.check_allowed(interaction):
            await deps.send_not_allowed(interaction)
            return
        await interaction.response.defer(thinking=True)
        await handler(interaction, prompt)

    @bot.tree.command(name="new", description="Create a new Codex thread with the first prompt.")
    async def slash_new(interaction: PromptSlashInteraction, prompt: str) -> None:
        await run_prompt_command(interaction, prompt, deps.handle_new)

    @bot.tree.command(name="ask", description="Send a prompt to the mapped or selected Codex thread.")
    async def slash_ask(interaction: PromptSlashInteraction, prompt: str) -> None:
        await run_prompt_command(interaction, prompt, deps.handle_ask)

    @bot.tree.command(name="interview", description="Clarify a request before implementation.")
    async def slash_interview(interaction: PromptSlashInteraction, prompt: str) -> None:
        await run_prompt_command(interaction, prompt, deps.handle_interview)

    @bot.tree.command(name="ask_ipc", description="Legacy alias of /ask.")
    async def slash_ask_ipc(interaction: PromptSlashInteraction, prompt: str) -> None:
        await run_prompt_command(interaction, prompt, deps.handle_ask)

    _ = (
        slash_new,
        slash_ask,
        slash_interview,
        slash_ask_ipc,
    )
