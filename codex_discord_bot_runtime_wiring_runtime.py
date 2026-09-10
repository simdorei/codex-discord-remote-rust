from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import TypeAlias, cast

import codex_discord_bot_command_wiring_runtime as discord_bot_command_wiring_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_history_runtime as discord_bot_history_runtime
import codex_discord_bot_message_slash_lifecycle_wiring_runtime as discord_bot_message_lifecycle_wiring_runtime
import codex_discord_bot_mirror_status_wiring_runtime as discord_bot_mirror_status_wiring_runtime
import codex_discord_bot_prompt_flow_wiring_runtime as discord_bot_prompt_flow_wiring_runtime
import codex_discord_bot_prompt_wiring_runtime as discord_bot_prompt_wiring_runtime
import codex_discord_bot_service_wiring_runtime as discord_bot_service_wiring_runtime
import codex_discord_bot_session_runner_wiring_runtime as discord_bot_session_runner_wiring_runtime
import codex_discord_diagnostics_history as discord_diagnostics_history
import codex_discord_steering as discord_steering

ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotRuntimeWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_history_runtime()
        self._set("SteeringPromptResult", discord_steering.SteeringPromptResult)
        self._install_prompt_wiring_runtime()
        self._install_service_wiring_runtime()
        self._install_command_wiring_runtime()
        self._install_mirror_status_wiring_runtime()
        self._install_session_runner_wiring_runtime()
        self._install_prompt_flow_wiring_runtime()
        self._install_message_slash_lifecycle_wiring_runtime()

    def _install_history_runtime(self) -> None:
        runtime = discord_bot_history_runtime.BotHistoryRuntime(
            discord_bot_history_runtime.BotHistoryRuntimeDeps(
                history_limit=cast(int, getattr(self.module, "HISTORY_POLL_HISTORY_LIMIT")),
                target_limit=50,
                delivery_exceptions=cast(
                    tuple[type[BaseException], ...],
                    getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS"),
                ),
                get_targets=self._get_startup_probe_targets,
                claim_message=self._claim_discord_message,
                mark_processed=self._mark_discord_message_processed,
                process_history_poll_message=self._process_history_poll_message,
                format_log_text_len=cast(Callable[[str | None], int | str], self._module_func("format_log_text_len")),
                log=self._log,
            )
        )
        self._set("HISTORY_RUNTIME", runtime)

    def _install_prompt_wiring_runtime(self) -> None:
        runtime = discord_bot_prompt_wiring_runtime.BotPromptWiringRuntime(module=self.module)
        self._set("PROMPT_WIRING_RUNTIME", runtime)
        runtime.install()

    def _install_service_wiring_runtime(self) -> None:
        runtime = discord_bot_service_wiring_runtime.BotServiceWiringRuntime(module=self.module)
        self._set("SERVICE_WIRING_RUNTIME", runtime)
        runtime.install()

    def _install_command_wiring_runtime(self) -> None:
        runtime = discord_bot_command_wiring_runtime.BotCommandWiringRuntime(module=self.module)
        self._set("COMMAND_WIRING_RUNTIME", runtime)
        runtime.install()

    def _install_mirror_status_wiring_runtime(self) -> None:
        runtime = discord_bot_mirror_status_wiring_runtime.BotMirrorStatusWiringRuntime(module=self.module)
        self._set("MIRROR_STATUS_WIRING_RUNTIME", runtime)
        runtime.install()

    def _install_session_runner_wiring_runtime(self) -> None:
        runtime = discord_bot_session_runner_wiring_runtime.BotSessionRunnerWiringRuntime(module=self.module)
        self._set("SESSION_RUNNER_WIRING_RUNTIME", runtime)
        runtime.install()

    def _install_prompt_flow_wiring_runtime(self) -> None:
        runtime = discord_bot_prompt_flow_wiring_runtime.BotPromptFlowWiringRuntime(module=self.module)
        self._set("PROMPT_FLOW_WIRING_RUNTIME", runtime)
        runtime.install()

    def _install_message_slash_lifecycle_wiring_runtime(self) -> None:
        runtime = discord_bot_message_lifecycle_wiring_runtime.BotMessageSlashLifecycleWiringRuntime(
            module=self.module
        )
        self._set("MESSAGE_SLASH_LIFECYCLE_WIRING_RUNTIME", runtime)
        runtime.install()

    def _get_startup_probe_targets(
        self,
        allowed_channel_ids: set[int],
        startup_channel_id: int | None,
        *,
        limit: int = 50,
    ) -> list[tuple[str, int]]:
        return cast(
            Callable[..., list[tuple[str, int]]],
            self._module_func("get_startup_probe_targets"),
        )(allowed_channel_ids, startup_channel_id, limit=limit)

    def _claim_discord_message(
        self,
        owner: discord_bot_history_runtime.HistoryPollOwner,
        message: discord_diagnostics_history.DiscordHistoryMessage,
    ) -> bool:
        return cast(Callable[..., bool], self._module_func("claim_discord_message"))(owner, message)

    def _mark_discord_message_processed(
        self,
        owner: discord_bot_history_runtime.HistoryPollOwner,
        message: discord_diagnostics_history.DiscordHistoryMessage,
    ) -> None:
        _ = cast(Callable[..., ModuleValue], self._module_func("mark_discord_message_processed"))(
            owner,
            message,
        )

    async def _process_history_poll_message(
        self,
        owner: discord_bot_history_runtime.HistoryPollOwner,
        message: discord_diagnostics_history.DiscordHistoryMessage,
        channel_id: int,
    ) -> None:
        await cast(
            Callable[..., Awaitable[None]],
            self._module_attr("CodexDiscordBot", "process_history_poll_message"),
        )(
            owner,
            message,
            channel_id,
        )

    def _module_attr(self, module_attr_name: str, attr_name: str) -> ModuleValue:
        module_attr = cast(ModuleValue, getattr(self.module, module_attr_name))
        return cast(ModuleValue, getattr(module_attr, attr_name))

    def _module_func(self, name: str) -> ModuleValue:
        return cast(ModuleValue, getattr(self.module, name))

    def _log(self, message: str) -> None:
        cast(Callable[[str], None], self._module_func("log_line"))(message)

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )
