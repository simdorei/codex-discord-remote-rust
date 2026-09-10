from __future__ import annotations

from dataclasses import dataclass
from typing import Generic, TypeVar

import codex_discord_prefix_prompt_commands as discord_prefix_prompt_commands
import codex_discord_slash_prompt_commands as discord_slash_prompt_commands

SourceMessageT = TypeVar("SourceMessageT")


@dataclass(frozen=True, slots=True)
class BotSkillSlashRuntimeDeps(Generic[SourceMessageT]):
    prompt_deps: discord_slash_prompt_commands.SkillSlashPromptDeps[SourceMessageT]


@dataclass(frozen=True, slots=True)
class BotSkillSlashRuntime(Generic[SourceMessageT]):
    deps: BotSkillSlashRuntimeDeps[SourceMessageT]

    async def handle_interview(
        self,
        interaction: discord_slash_prompt_commands.SkillSlashInteraction,
        prompt: str,
    ) -> None:
        await self._handle(
            interaction,
            prompt,
            spec=discord_slash_prompt_commands.SkillSlashPromptSpec(
                title="Interview",
                log_name="slash_interview",
                ack_message="Interview handling posted in this channel.",
                ack_context="interview_posted",
                build_prompt=discord_prefix_prompt_commands.build_deep_interview_prompt,
            ),
        )

    async def _handle(
        self,
        interaction: discord_slash_prompt_commands.SkillSlashInteraction,
        prompt: str,
        *,
        spec: discord_slash_prompt_commands.SkillSlashPromptSpec,
    ) -> None:
        await discord_slash_prompt_commands.handle_skill_slash_prompt(
            interaction,
            prompt,
            spec=spec,
            deps=self.deps.prompt_deps,
        )
