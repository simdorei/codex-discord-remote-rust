from __future__ import annotations

import traceback

from codex_discord_slash_commands import SlashCallback, SlashCommandBot
from codex_discord_slash_runtime_types import (
    QueueRetractResult,
    RuntimeBridgeSessionRefresher,
    RuntimeButtonQaRunner,
    RuntimeDoctorMessageBuilder,
    RuntimeQueueRetractor,
    RuntimeSlashBridgeRunner,
    RuntimeSlashChannel,
    RuntimeSlashChunksSender,
    RuntimeSlashCommandDeps,
    RuntimeSlashInteraction,
    RuntimeSlashResponseSender,
    RuntimeSlashSourceMessage,
    RuntimeSlashUser,
)

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
    "register_runtime_slash_commands",
]


async def _prepare_runtime_slash_interaction(
    interaction: RuntimeSlashInteraction,
    deps: RuntimeSlashCommandDeps,
    *,
    defer: bool = True,
) -> bool:
    if not deps.check_allowed(interaction):
        await deps.send_not_allowed(interaction)
        return False
    if defer:
        await interaction.response.defer(thinking=True)
    return True


def register_runtime_slash_commands(
    bot: SlashCommandBot,
    deps: RuntimeSlashCommandDeps,
) -> None:
    callbacks: list[SlashCallback] = []

    @bot.tree.command(name="doctor", description="Run Codex bridge diagnostics.")
    async def slash_doctor(interaction: RuntimeSlashInteraction) -> None:
        if not await _prepare_runtime_slash_interaction(interaction, deps):
            return
        await deps.send_chunks(
            interaction,
            await deps.build_doctor(bot, interaction.channel_id, interaction.channel),
            title="Discord doctor",
        )
        _ = await deps.run_bridge(interaction, ["doctor"], "Doctor")

    @bot.tree.command(name="runners", description="Show Discord runner queues.")
    async def slash_runners(interaction: RuntimeSlashInteraction) -> None:
        if not await _prepare_runtime_slash_interaction(interaction, deps):
            return
        await deps.send_chunks(interaction, await deps.build_runners(), title="Runners")

    @bot.tree.command(name="retract", description="Remove your latest queued ask for this Codex thread.")
    async def slash_retract(interaction: RuntimeSlashInteraction, ref: str = "") -> None:
        if not await _prepare_runtime_slash_interaction(interaction, deps):
            return
        response, _result = await deps.retract_queued_ask(
            channel_id=interaction.channel_id,
            user_id=interaction.user.id,
            ref=ref or None,
        )
        await deps.send_chunks(interaction, response, title="Retract")

    @bot.tree.command(name="mirror_check", description="Check Discord mirror mappings.")
    async def slash_mirror_check(interaction: RuntimeSlashInteraction) -> None:
        if not await _prepare_runtime_slash_interaction(interaction, deps):
            return
        try:
            output = await deps.run_mirror_check()
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK - slash boundary reports failures to Discord.
            deps.log_line("slash_mirror_check_failed\n" + traceback.format_exc())
            output = f"Mirror check failed\n\nERROR: {exc}"
        await deps.send_chunks(interaction, output, title="Mirror check")

    @bot.tree.command(name="bridge_sync", description="Refresh Codex bridge state and Discord mirror.")
    async def slash_bridge_sync(
        interaction: RuntimeSlashInteraction,
        limit: int | None = None,
    ) -> None:
        if not await _prepare_runtime_slash_interaction(interaction, deps):
            return
        try:
            output = await deps.refresh_bridge_session(bot, limit=limit)
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK - slash boundary reports failures to Discord.
            deps.log_line("slash_bridge_sync_failed\n" + traceback.format_exc())
            output = f"Discord bridge sync failed\n\nERROR: {exc}"
        await deps.send_chunks(interaction, output, title="Bridge sync")

    callbacks.extend(
        [
            slash_doctor,
            slash_runners,
            slash_retract,
            slash_mirror_check,
            slash_bridge_sync,
        ]
    )

    if deps.qa_commands_enabled():

        @bot.tree.command(name="qa_buttons", description="Run Discord button QA smoke.")
        async def slash_qa_buttons(interaction: RuntimeSlashInteraction) -> None:
            if not await _prepare_runtime_slash_interaction(interaction, deps, defer=False):
                return
            if interaction.channel is None:
                await deps.send_response(
                    interaction,
                    "Discord channel is unavailable.",
                    ephemeral=True,
                    context="qa_buttons_channel_unavailable",
                )
                return
            await interaction.response.defer(thinking=True)
            source_message = RuntimeSlashSourceMessage(author=interaction.user, channel=interaction.channel)
            output = await deps.run_button_qa(bot, source_message)
            await deps.send_chunks(interaction, output, title="Discord button QA")

        callbacks.append(slash_qa_buttons)

    _ = callbacks
