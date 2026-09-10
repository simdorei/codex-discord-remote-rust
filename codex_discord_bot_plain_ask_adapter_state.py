from __future__ import annotations

from collections.abc import Callable
from types import ModuleType
from typing import Protocol, cast

from codex_discord_bot_plain_ask_adapter_types import ModuleValue
import codex_discord_plain_ask as discord_plain_ask
import codex_discord_plain_ask_handler as discord_plain_ask_handler


class BotPlainAskAdapterStateMixin(Protocol):
    module: ModuleType

    def has_recent_codex_app_user_prompt(self, target_thread_id: str | None, prompt: str) -> bool:
        return cast(
            discord_plain_ask_handler.HasRecentPromptSyncFunc,
            self._module_func("has_recent_codex_app_user_prompt"),
        )(target_thread_id, prompt)

    async def is_thread_runner_busy(self, target_thread_id: str | None) -> bool:
        return await cast(discord_plain_ask.IsRunnerBusyFunc, self._module_func("is_thread_runner_busy"))(
            target_thread_id
        )

    def mark_recent_discord_origin_prompt(self, target_thread_id: str | None, prompt: str) -> None:
        cast(Callable[[str | None, str], None], self._module_func("mark_recent_discord_origin_prompt"))(
            target_thread_id,
            prompt,
        )

    async def claim_direct_ask_target(self, target_thread_id: str | None) -> bool:
        return await cast(discord_plain_ask.ClaimDirectAskTargetFunc, self._module_func("claim_direct_ask_target"))(
            target_thread_id
        )

    async def release_direct_ask_target(self, target_thread_id: str | None) -> None:
        await cast(discord_plain_ask.ReleaseDirectAskTargetFunc, self._module_func("release_direct_ask_target"))(
            target_thread_id
        )

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
