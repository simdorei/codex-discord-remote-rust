from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast, TypeAlias

import codex_discord_busy as discord_busy
import codex_discord_busy_interaction_runtime as discord_busy_interaction_runtime
import codex_discord_delivery_state as discord_delivery_state
import codex_discord_prompt_busy_result as discord_prompt_busy_result
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotBusyInteractionAdapterRuntime:
    module: ModuleType

    def make_busy_interaction_runtime(self) -> discord_busy_interaction_runtime.BusyInteractionRuntime:
        return discord_busy_interaction_runtime.BusyInteractionRuntime(
            send_direct_followup=self.send_direct_followup,
            send_stale_busy_steer_block_message=self.send_stale_busy_steer_block_message,
            send_codex_app_menu_if_available=self.send_codex_app_menu_if_available,
            send_steering_start_ack=self.send_steering_start_ack,
        )

    async def send_direct_followup(
        self,
        interaction: discord_delivery_state.InteractionLike,
        content: str,
        *,
        log_prefix: str,
        context: str,
    ) -> None:
        await cast(
            discord_busy_interaction_runtime.DirectFollowupSender,
            self._module_func("send_direct_followup"),
        )(interaction, content, log_prefix=log_prefix, context=context)

    async def send_stale_busy_steer_block_message(
        self,
        channel: discord_delivery_state.Messageable,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> bool:
        return await cast(
            discord_busy_interaction_runtime.StaleBlockSender,
            self._module_func("send_stale_busy_steer_block_message"),
        )(channel, prompt, target_thread_id, reason=reason)

    async def send_codex_app_menu_if_available(
        self,
        channel: discord_delivery_state.Messageable,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> bool:
        return await cast(
            discord_busy_interaction_runtime.CodexAppMenuSender,
            self._module_func("send_codex_app_menu_if_available"),
        )(channel, target_thread_id, output, reason=reason)

    async def send_steering_start_ack(
        self,
        channel: discord_delivery_state.Messageable,
        prompt: str,
        target_thread_id: str | None,
    ) -> bool:
        return await cast(
            discord_busy_interaction_runtime.SteeringStartAckSender,
            self._module_func("send_steering_start_ack"),
        )(channel, prompt, target_thread_id)

    def build_codex_app_steering_not_accepted_message(self, target_ref: str) -> str:
        return discord_prompt_busy_result.build_codex_app_steering_not_accepted_message(target_ref)

    def build_busy_choice_message(self, prompt: str, target_thread_id: str | None) -> str:
        return discord_busy.build_busy_choice_message(
            prompt,
            target_thread_id,
            discord_max_len=cast(int, getattr(self.module, "DISCORD_MAX_LEN")),
            fit_single_message_func=cast(Callable[[str], str], self._module_func("fit_single_message")),
        )

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
