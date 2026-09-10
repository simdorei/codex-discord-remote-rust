from __future__ import annotations

from collections.abc import Callable
from contextlib import AbstractContextManager
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import codex_app_server_transport as app_server_transport
import codex_app_server_transport_delivery as app_server_delivery
import codex_discord_ask_stream_factory as discord_ask_stream_factory
import codex_discord_bot_prompt_resume_adapter as discord_prompt_resume_adapter
import codex_discord_bot_prompt_transport_preprocess as discord_prompt_transport_preprocess
import codex_discord_bot_prompt_transport_runtime as discord_bot_prompt_transport_runtime
import codex_discord_prompt_busy_result as discord_prompt_busy_result
import codex_discord_prompt_mapped_delivery as discord_prompt_mapped_delivery
import codex_discord_prompt_mapped_delivery_factory as discord_prompt_mapped_delivery_factory
import codex_discord_prompt_transport as discord_prompt_transport
import codex_discord_stream as discord_stream
ModuleValue: TypeAlias = object


PromptChannel: TypeAlias = object
SteeringResult: TypeAlias = object


class SelectedThreadBridge(Protocol):
    def set_selected_thread_id(self, thread_id: str) -> None: ...


@dataclass(frozen=True, slots=True)
class BotPromptTransportAdapterRuntime:
    module: ModuleType

    def make_prompt_transport_runtime(
        self,
    ) -> discord_bot_prompt_transport_runtime.BotPromptTransportRuntime[
        PromptChannel,
        discord_stream.DiscordAskRelay,
        SteeringResult,
    ]:
        return discord_bot_prompt_transport_runtime.BotPromptTransportRuntime(
            discord_bot_prompt_transport_runtime.BotPromptTransportRuntimeDeps(
                bridge_module=cast(
                    app_server_delivery.BridgeModule,
                    getattr(self.module, "BRIDGE_APP_SERVER_DELIVERY"),
                ),
                app_server_transport_enabled=cast(
                    discord_prompt_transport.TransportEnabled,
                    self._module_func("app_server_transport_enabled"),
                ),
                run_legacy_prompt_no_wait=self.run_legacy_prompt_no_wait,
                make_steering_prompt_result=cast(
                    discord_prompt_transport.MakeSteeringResult[
                        app_server_transport.AppServerDeliveryResult,
                        SteeringResult,
                    ],
                    self._module_func("make_app_server_steering_result"),
                ),
                run_watch_stream=self.run_watch_stream,
                run_bridge_command_stream=self.run_bridge_command_stream,
                ui_fallback_lock=cast(
                    AbstractContextManager[bool],
                    getattr(self.module, "UI_FALLBACK_LOCK"),
                ),
                preprocess_prompt=discord_prompt_transport_preprocess.make_prompt_preprocessor(self.module),
                mark_recent_discord_origin_prompt=discord_prompt_transport_preprocess.make_discord_origin_prompt_marker(
                    self.module
                ),
                prepare_mapped_session_mirror_output=cast(
                    discord_prompt_mapped_delivery.PrepareMappedSessionMirrorOutput[PromptChannel],
                    self._module_func("prepare_mapped_session_mirror_output"),
                ),
                set_selected_thread_id=self.set_selected_thread_id,
                channel_typing=cast(
                    discord_prompt_mapped_delivery.ChannelTyping[PromptChannel],
                    self._module_func("mapped_prompt_delivery_channel_typing"),
                ),
                run_transport_prompt_no_wait=self.run_transport_prompt_no_wait,
                send_chunks=cast(
                    discord_prompt_mapped_delivery.ChunkSender[PromptChannel],
                    self._module_func("send_prompt_chunks"),
                ),
                is_delivery_confirmation_timeout=self.is_delivery_confirmation_timeout,
                format_pending_ask_delivery_output=self.format_pending_ask_delivery_output,
                release_session_mirror_output_target=cast(
                    discord_prompt_mapped_delivery.OutputTargetReleaser,
                    self._module_func("release_session_mirror_output_target"),
                ),
                is_selected_thread_busy_error=self.is_selected_thread_busy_error,
                send_codex_app_menu_if_available=self.send_codex_app_menu_if_available,
                send_resume_failure=self.send_resume_failure,
                handle_recorded_busy_transport_prompt=self.handle_recorded_busy_transport_prompt,
                wait_for_mirrored_busy_delegation_settle=self.wait_for_mirrored_busy_delegation_settle,
                mark_steering_handoff=cast(
                    discord_prompt_busy_result.SteeringHandoffMarker,
                    self._module_func("mark_busy_steering_handoff"),
                ),
                get_relay_factory=self.get_relay_factory,
                get_run_ask_stream=self.get_run_ask_stream,
                format_log_text_len=cast(
                    discord_prompt_busy_result.TextLenFunc,
                    self._module_func("format_log_text_len"),
                ),
                log=cast(discord_prompt_busy_result.LogFunc, self._module_func("log_line")),
            )
        )

    def run_legacy_prompt_no_wait(self, prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        return cast(
            discord_prompt_transport.PromptNoWait,
            self._module_func("run_legacy_ipc_prompt_no_wait"),
        )(prompt, target_thread_id)

    def run_transport_prompt_no_wait(self, prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        return cast(
            discord_prompt_mapped_delivery_factory.SyncTransportNoWait,
            self._module_func("run_transport_prompt_no_wait"),
        )(prompt, target_thread_id)

    def set_selected_thread_id(self, thread_id: str) -> None:
        bridge = cast(SelectedThreadBridge, getattr(self.module, "BRIDGE_SELECTED_THREAD"))
        bridge.set_selected_thread_id(thread_id)

    def run_watch_stream(
        self,
        steering_result: SteeringResult,
        relay: discord_stream.DiscordAskRelay,
    ) -> tuple[int, str]:
        return cast(
            discord_prompt_transport.WatchStream[SteeringResult, discord_stream.DiscordAskRelay],
            self._module_func("run_steering_watch_stream"),
        )(steering_result, relay)

    def run_bridge_command_stream(
        self,
        argv: list[str],
        on_line: Callable[[str], None],
    ) -> tuple[int, str]:
        return cast(
            discord_stream.RunBridgeCommandStreamFunc,
            self._module_func("run_bridge_command_stream"),
        )(argv, on_line)

    def is_delivery_confirmation_timeout(self, output: str) -> bool:
        return cast(
            discord_prompt_mapped_delivery.OutputPredicate,
            self._module_attr("discord_steering", "is_ipc_delivery_confirmation_timeout"),
        )(output)

    def format_pending_ask_delivery_output(self, output: str) -> str:
        return cast(
            discord_prompt_mapped_delivery.PendingFormatter,
            self._module_attr("discord_prompt_pending_delivery", "format_pending_ask_delivery_output"),
        )(output)

    def is_selected_thread_busy_error(self, exit_code: int, output: str) -> bool:
        return cast(
            discord_prompt_mapped_delivery.BusyPredicate,
            self._module_attr("discord_busy", "is_selected_thread_busy_error"),
        )(exit_code, output)

    async def send_codex_app_menu_if_available(
        self,
        channel: PromptChannel,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> bool:
        return await cast(
            discord_prompt_mapped_delivery.AppMenuSender[PromptChannel],
            self._module_func("send_codex_app_menu_if_available"),
        )(
            channel,
            target_thread_id,
            output,
            reason=reason,
        )

    async def send_resume_failure(
        self,
        channel: PromptChannel,
        content: str,
        target_thread_id: str,
    ) -> None:
        await discord_prompt_resume_adapter.send_resume_failure(
            self.module,
            channel,
            content,
            target_thread_id,
        )

    async def handle_recorded_busy_transport_prompt(
        self,
        channel: PromptChannel,
        prompt: str,
        *,
        target_thread_id: str | None,
        target_ref: str,
        recent_offsets: discord_prompt_busy_result.RecentOffsets,
        transport_output: str,
        delegate_to_session_mirror: bool,
    ) -> bool:
        return await cast(
            discord_prompt_busy_result.RecordedBusyHandler[PromptChannel],
            self._module_func("handle_recorded_busy_transport_prompt"),
        )(
            channel,
            prompt,
            target_thread_id=target_thread_id,
            target_ref=target_ref,
            recent_offsets=recent_offsets,
            transport_output=transport_output,
            delegate_to_session_mirror=delegate_to_session_mirror,
        )

    async def wait_for_mirrored_busy_delegation_settle(
        self,
        prompt: str,
        *,
        target_thread_id: str | None,
        recent_offsets: discord_prompt_busy_result.RecentOffsets,
    ) -> None:
        await cast(
            discord_prompt_busy_result.BusySettleWaiter,
            self._module_func("wait_for_mirrored_busy_delegation_settle"),
        )(
            prompt,
            target_thread_id=target_thread_id,
            recent_offsets=recent_offsets,
        )

    def get_relay_factory(
        self,
    ) -> discord_ask_stream_factory.DiscordAskRelayFactory[
        PromptChannel,
        discord_stream.DiscordAskRelay,
    ]:
        return cast(
            discord_ask_stream_factory.DiscordAskRelayFactory[
                PromptChannel,
                discord_stream.DiscordAskRelay,
            ],
            getattr(self.module, "DiscordAskRelay"),
        )

    def get_run_ask_stream(self) -> discord_ask_stream_factory.RunAskStreamFunc[discord_stream.DiscordAskRelay]:
        return cast(
            discord_ask_stream_factory.RunAskStreamFunc[discord_stream.DiscordAskRelay],
            self._module_func("run_ask_stream"),
        )

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))

    def _module_attr(self, module_name: str, attr_name: str) -> ModuleValue:
        imported_module = cast(ModuleType, getattr(self.module, module_name))
        return cast(object, getattr(imported_module, attr_name))
