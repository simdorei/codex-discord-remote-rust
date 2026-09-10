from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from typing import cast

import discord
from discord import app_commands

import codex_discord_commands as discord_commands
import codex_discord_settings_commands as settings_commands
from codex_discord_slash_types import (
    BasicSlashCommandDeps,
    BasicSlashInteraction,
    ContextMessageBuilder,
    ContextRefreshMessageBuilder,
    InteractionBridgeRunner,
    InteractionChunksSender,
    InteractionResponse,
    SlashCallback,
    SlashCommandBot,
    SlashCommandTree,
    SlashInteraction,
    SettingsModelCatalog,
    SettingsModelCatalogLoader,
    WeeklyUsageMessageBuilder,
)

__all__ = [
    "BasicSlashCommandDeps",
    "BasicSlashInteraction",
    "ContextMessageBuilder",
    "ContextRefreshMessageBuilder",
    "InteractionBridgeRunner",
    "InteractionChunksSender",
    "InteractionResponse",
    "SlashCallback",
    "SlashCommandBot",
    "SlashCommandTree",
    "SlashInteraction",
    "SettingsModelCatalog",
    "SettingsModelCatalogLoader",
    "WeeklyUsageMessageBuilder",
    "register_basic_slash_commands",
]


async def _prepare_basic_slash_interaction(
    interaction: SlashInteraction,
    deps: BasicSlashCommandDeps,
) -> bool:
    if not deps.check_allowed(interaction):
        await deps.send_not_allowed(interaction)
        return False
    await interaction.response.defer(thinking=True)
    return True


def register_basic_slash_commands(bot: SlashCommandBot, deps: BasicSlashCommandDeps) -> None:
    async def settings_model_autocomplete(
        interaction: discord.Interaction,
        current: str,
    ) -> list[app_commands.Choice[str]]:
        if not deps.check_allowed(interaction):
            return []
        catalog = await asyncio.to_thread(deps.load_settings_model_catalog)
        return [
            app_commands.Choice(name=value, value=value)
            for value in settings_commands.build_model_autocomplete_values(catalog, current)
        ]

    async def settings_effort_autocomplete(
        interaction: discord.Interaction,
        current: str,
    ) -> list[app_commands.Choice[str]]:
        if not deps.check_allowed(interaction):
            return []
        catalog = await asyncio.to_thread(deps.load_settings_model_catalog)
        selected_model = cast(str | None, interaction.namespace.model)
        return [
            app_commands.Choice(name=value, value=value)
            for value in settings_commands.build_effort_autocomplete_values(
                catalog,
                current,
                model=selected_model,
            )
        ]

    @bot.tree.command(name="help", description="Show Discord Codex commands.")
    async def slash_help(interaction: SlashInteraction) -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        await deps.send_chunks(interaction, deps.build_help(), title="Help")

    @bot.tree.command(name="list", description="Show recent Codex threads.")
    async def slash_list(interaction: SlashInteraction, limit: int = 10) -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        _ = await deps.run_bridge(
            interaction,
            discord_commands.build_list_argv(limit, default=10, maximum=30),
            "List",
        )

    @bot.tree.command(name="archived_list", description="Show archived Codex threads.")
    async def slash_archived_list(interaction: SlashInteraction, limit: int = 10) -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        _ = await deps.run_bridge(
            interaction,
            discord_commands.build_archived_list_argv(limit, default=10, maximum=50),
            "Archived list",
        )

    @bot.tree.command(name="use", description="Select the active Codex thread.")
    async def slash_use(interaction: SlashInteraction, ref: str) -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        _ = await deps.run_bridge(interaction, ["use", ref], "Use")

    @bot.tree.command(name="status", description="Show selected Codex thread status.")
    async def slash_status(interaction: SlashInteraction, ref: str = "") -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        argv = discord_commands.build_status_argv(
            interaction.channel_id,
            ref or None,
            resolve_target_args_func=deps.resolve_target_args,
        )
        _ = await deps.run_bridge(interaction, argv, "Status")

    @bot.tree.command(name="settings", description="Update Codex thread model, effort, or speed.")
    @app_commands.autocomplete(
        model=settings_model_autocomplete,
        effort=settings_effort_autocomplete,
    )
    async def slash_settings(
        interaction: SlashInteraction,
        ref: str = "",
        model: str = "",
        effort: str = "",
        speed: str = "",
    ) -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        action = discord_commands.build_settings_values_argv(
            interaction.channel_id,
            ref or None,
            model=model,
            effort=effort,
            speed=speed,
            resolve_target_args_func=deps.resolve_target_args,
        )
        if action.argv is None:
            await deps.send_chunks(
                interaction,
                action.usage or discord_commands.SETTINGS_USAGE,
                title=action.title,
            )
            return
        _ = await deps.run_bridge(interaction, action.argv, action.title)

    @bot.tree.command(name="where", description="Show the Codex thread mapped to this Discord channel.")
    async def slash_where(interaction: SlashInteraction) -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        await deps.send_chunks(interaction, deps.build_where(interaction.channel_id), title="Where")

    @bot.tree.command(name="context", description="Show context usage for this Codex thread.")
    async def slash_context(
        interaction: SlashInteraction,
        all_threads: bool = False,
        refresh: bool = False,
        limit: int = 10,
    ) -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        if refresh:
            output = deps.build_context_refresh(
                interaction.channel_id,
                limit=deps.clamp_context_refresh_limit(limit),
            )
        else:
            output = deps.build_context(interaction.channel_id, all_threads=all_threads, limit=20)
        await deps.send_chunks(interaction, output, title="Context")

    @bot.tree.command(name="usage", description="Show live Codex usage and rate limits.")
    async def slash_usage(interaction: SlashInteraction, days: int = 7) -> None:
        if not await _prepare_basic_slash_interaction(interaction, deps):
            return
        output = await asyncio.to_thread(
            deps.build_weekly_usage,
            days=max(1, min(30, days)),
        )
        await deps.send_chunks(interaction, output, title="Usage")

    _ = (
        slash_help,
        slash_list,
        slash_archived_list,
        slash_use,
        slash_status,
        slash_settings,
        slash_where,
        slash_context,
        slash_usage,
    )
