from __future__ import annotations

import importlib
import traceback
from contextlib import AbstractContextManager
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import codex_discord_bot_main_runtime as discord_bot_main_runtime
import codex_discord_delivery as discord_delivery
import codex_discord_diagnostics_history as discord_diagnostics_history
import codex_discord_interaction_gate as discord_interaction_gate
import codex_discord_interaction_gate_runtime as discord_interaction_gate_runtime
import codex_discord_slash_error as discord_slash_error


SteeringPromptResult: TypeAlias = object
MessageableChannel: TypeAlias = object
ModuleValue: TypeAlias = object


class RecordedBusyStreamer(Protocol):
    def __call__(
        self,
        channel: MessageableChannel,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        label: str,
    ) -> Awaitable[bool]: ...


class RuntimeDoctorBuilder(Protocol):
    def __call__(
        self,
        bot: ModuleValue,
        channel_id: int | None,
        channel: ModuleValue | None,
    ) -> Awaitable[str]: ...


class RuntimeBridgeSessionRefresher(Protocol):
    def __call__(self, bot: ModuleValue, *, limit: int | None = None) -> Awaitable[str]: ...


@dataclass(frozen=True, slots=True)
class BotMiscAdapterRuntime:
    module: ModuleType

    def make_logging_command_tree_class(self) -> type[ModuleValue]:
        runtime = self
        app_commands = importlib.import_module("discord.app_commands")
        command_tree_base = cast(type[ModuleValue], getattr(app_commands, "CommandTree"))

        class LoggingCommandTree(command_tree_base):
            async def on_error(
                self,
                interaction: ModuleValue,
                error: BaseException,
                /,
            ) -> None:
                interaction_like = cast(discord_slash_error.SlashErrorInteraction, interaction)
                await discord_slash_error.handle_slash_command_error(
                    interaction_like,
                    error,
                    deps=discord_slash_error.SlashCommandErrorDeps(
                        get_command_name=cast(
                            Callable[[discord_slash_error.SlashErrorInteraction], str],
                            discord_delivery.get_interaction_command_name,
                        ),
                        delivery_rejected_type=cast(
                            type[BaseException],
                            getattr(runtime.module, "DiscordDeliveryRejected"),
                        ),
                        restarting_notice=cast(str, getattr(runtime.module, "DISCORD_RESTARTING_NOTICE")),
                        send_followup=cast(
                            discord_slash_error.FollowupSender[
                                discord_slash_error.SlashErrorInteraction,
                                ModuleValue,
                            ],
                            runtime._module_func("send_direct_followup"),
                        ),
                        send_initial_response=cast(
                            discord_slash_error.InitialResponseSender[
                                discord_slash_error.SlashErrorInteraction,
                            ],
                            runtime._module_func("send_interaction_response_tracked"),
                        ),
                        delivery_exceptions=cast(
                            tuple[type[BaseException], ...],
                            getattr(runtime.module, "DISCORD_DELIVERY_EXCEPTIONS"),
                        ),
                        format_exception=traceback.format_exc,
                        log=cast(Callable[[str], None], runtime._module_func("log_line")),
                    ),
                )

        return LoggingCommandTree

    def stream_recorded_busy_steering_result(
        self,
        channel: MessageableChannel,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        label: str,
    ) -> Awaitable[bool]:
        return cast(
            RecordedBusyStreamer,
            self._module_func("stream_steering_prompt_result_to_channel"),
        )(channel, steering_result, target_thread_id, label=label)

    def require_discord_interaction(self, interaction: ModuleValue) -> ModuleValue:
        return interaction

    def require_discord_messageable(self, channel: ModuleValue) -> MessageableChannel:
        return channel

    def require_discord_history_channel(
        self,
        channel: ModuleValue,
    ) -> discord_diagnostics_history.DiscordChannelLike:
        return cast(discord_diagnostics_history.DiscordChannelLike, channel)

    async def build_runtime_discord_doctor_message(
        self,
        bot: ModuleValue,
        channel_id: int | None,
        channel: ModuleValue | None,
    ) -> str:
        runtime_bot = cast(Callable[[ModuleValue], ModuleValue], self._module_func("require_runtime_codex_bot"))(bot)
        return await cast(
            RuntimeDoctorBuilder,
            self._module_func("build_discord_doctor_message_with_history"),
        )(runtime_bot, channel_id, channel)

    async def refresh_runtime_discord_bridge_session(
        self,
        bot: ModuleValue,
        *,
        limit: int | None = None,
    ) -> str:
        runtime_bot = cast(Callable[[ModuleValue], ModuleValue], self._module_func("require_runtime_codex_bot"))(bot)
        return await cast(
            RuntimeBridgeSessionRefresher,
            self._module_func("refresh_discord_bridge_session"),
        )(runtime_bot, limit=limit)

    def check_interaction_allowed(
        self,
        bot: discord_interaction_gate.InteractionGateBot,
        interaction: discord_interaction_gate.InteractionLike,
    ) -> bool:
        return discord_interaction_gate.check_interaction_allowed(
            discord_interaction_gate_runtime.InteractionGateBotAdapter(bot),
            interaction,
            log_func=cast(Callable[[str], None], self._module_func("log_line")),
            get_interaction_command_name_func=cast(
                discord_interaction_gate.CommandNameFunc,
                self._module_func("get_interaction_gate_command_name"),
            ),
            is_mirrored_channel_id_func=cast(
                discord_interaction_gate.MirroredChannelFunc,
                self._module_func("is_mirrored_interaction_channel_id"),
            ),
        )

    def main(self) -> int:
        return discord_bot_main_runtime.main(
            discord_bot_main_runtime.BotMainDeps(
                env_path=cast(Path, getattr(self.module, "ENV_PATH")),
                bot_factory=cast(
                    discord_bot_main_runtime.BotFactory,
                    getattr(self.module, "CodexDiscordBot"),
                ),
                acquire_runtime_instance_lock=cast(
                    Callable[[], AbstractContextManager[bool]],
                    self._module_func("acquire_runtime_instance_lock"),
                ),
            )
        )

    def _module_func(self, name: str) -> ModuleValue:
        return cast(ModuleValue, getattr(self.module, name))
