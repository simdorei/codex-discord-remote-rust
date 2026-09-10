from __future__ import annotations

from collections.abc import Awaitable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_delivery_state as discord_delivery_state


class DirectFollowupSender(Protocol):
    def __call__(
        self,
        interaction: discord_delivery_state.InteractionLike,
        content: str,
        *,
        log_prefix: str,
        context: str,
    ) -> Awaitable[None]: ...


class StaleBlockSender(Protocol):
    def __call__(
        self,
        channel: discord_delivery_state.Messageable,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


class CodexAppMenuSender(Protocol):
    def __call__(
        self,
        channel: discord_delivery_state.Messageable,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


class SteeringStartAckSender(Protocol):
    def __call__(
        self,
        channel: discord_delivery_state.Messageable,
        prompt: str,
        target_thread_id: str | None,
    ) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class BusyInteractionRuntime:
    send_direct_followup: DirectFollowupSender
    send_stale_busy_steer_block_message: StaleBlockSender
    send_codex_app_menu_if_available: CodexAppMenuSender
    send_steering_start_ack: SteeringStartAckSender

    async def send_busy_direct_followup(
        self,
        interaction: discord_delivery_state.InteractionLike,
        content: str,
        *,
        log_prefix: str,
        context: str,
    ) -> None:
        await self.send_direct_followup(
            interaction,
            content,
            log_prefix=log_prefix,
            context=context,
        )

    async def send_busy_stale_block_message(
        self,
        channel: discord_delivery_state.Messageable,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> bool:
        return await self.send_stale_busy_steer_block_message(
            channel,
            prompt,
            target_thread_id,
            reason=reason,
        )

    async def send_busy_codex_app_menu_if_available(
        self,
        channel: discord_delivery_state.Messageable,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> bool:
        return await self.send_codex_app_menu_if_available(
            channel,
            target_thread_id,
            output,
            reason=reason,
        )

    async def send_persistent_busy_steering_start_ack(
        self,
        channel: discord_delivery_state.Messageable,
        prompt: str,
        target_thread_id: str | None,
    ) -> None:
        _ = await self.send_steering_start_ack(channel, prompt, target_thread_id)
