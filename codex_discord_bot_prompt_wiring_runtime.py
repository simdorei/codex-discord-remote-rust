from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast, TypeAlias

from codex_app_server_transport_delivery import BridgeModule
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_prompt_bridge_adapter_runtime as discord_bot_prompt_bridge_adapter_runtime
import codex_discord_bot_prompt_delivery_bridge_adapter_runtime as discord_bot_prompt_delivery_bridge_adapter_runtime
import codex_discord_bot_prompt_relay_adapter_runtime as discord_bot_prompt_relay_adapter_runtime
import codex_discord_bot_prompt_watch_adapter_runtime as discord_bot_prompt_watch_adapter_runtime
import codex_discord_bot_prompt_watch_approval_adapter_runtime as discord_bot_prompt_watch_approval_adapter_runtime
import codex_discord_prompt_relay_channels as discord_prompt_relay_channels
import codex_discord_stream as discord_stream
import codex_discord_stream_relay as discord_stream_relay
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotPromptWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_prompt_bridge()
        self._install_prompt_relay()
        self._install_prompt_watch()

    def _install_prompt_bridge(self) -> None:
        prompt_delivery_bridge = discord_bot_prompt_delivery_bridge_adapter_runtime.PromptDeliveryBridgeAdapter(
            cast(BridgeModule, getattr(self.module, "BRIDGE_APP_SERVER_DELIVERY"))
        )
        self._set("PROMPT_DELIVERY_BRIDGE", prompt_delivery_bridge)
        bridge_adapter_runtime = discord_bot_prompt_bridge_adapter_runtime.BotPromptBridgeAdapterRuntime(module=self.module)
        self._set("PROMPT_BRIDGE_ADAPTER_RUNTIME", bridge_adapter_runtime)
        bridge_runtime = bridge_adapter_runtime.make_prompt_bridge_runtime()
        self._set("PROMPT_BRIDGE_RUNTIME", bridge_runtime)
        self._set("get_bridge_script_path", bridge_runtime.get_bridge_script_path)
        self._set("run_bridge_command_stream", bridge_runtime.run_bridge_command_stream)
        self._set("run_ask", bridge_runtime.run_ask)
        self._set("app_server_transport_enabled", bridge_runtime.app_server_transport_enabled)
        self._set("run_legacy_ipc_prompt_no_wait", bridge_runtime.run_legacy_ipc_prompt_no_wait)
        self._set("run_transport_prompt_no_wait", bridge_runtime.run_transport_prompt_no_wait)
        self._set("run_resident_app_server_steering_prompt", bridge_runtime.run_resident_app_server_steering_prompt)
        self._set("submit_approval_reply", bridge_runtime.submit_approval_reply)
        self._set("submit_input_reply", bridge_runtime.submit_input_reply)
        self._set("run_steering_prompt", bridge_runtime.run_steering_prompt)

    def _install_prompt_relay(self) -> None:
        relay_adapter_runtime = discord_bot_prompt_relay_adapter_runtime.BotPromptRelayAdapterRuntime(module=self.module)
        self._set("PROMPT_RELAY_ADAPTER_RUNTIME", relay_adapter_runtime)
        self._set("PROMPT_RELAY_CHANNELS", relay_adapter_runtime.make_prompt_relay_channels())
        self._set("discord_stream", discord_stream)
        self._set("discord_stream_relay", discord_stream_relay)
        self._set("DiscordMessageableChannelTypeError", discord_prompt_relay_channels.DiscordMessageableChannelTypeError)
        self._set("require_discord_messageable_channel", relay_adapter_runtime.require_discord_messageable_channel)
        self._set("send_relay_chunks", relay_adapter_runtime.send_relay_chunks)
        self._set("send_relay_interactive_prompt", relay_adapter_runtime.send_relay_interactive_prompt)
        self._set("format_relay_log_text_len", relay_adapter_runtime.format_relay_log_text_len)
        self._set("format_log_text_len_as_text", relay_adapter_runtime.format_log_text_len_as_text)
        self._set("send_prompt_chunks", relay_adapter_runtime.send_prompt_chunks)
        self._set("DiscordAskRelay", relay_adapter_runtime.make_discord_ask_relay_class())

    def _install_prompt_watch(self) -> None:
        watch_adapter_runtime = discord_bot_prompt_watch_adapter_runtime.BotPromptWatchAdapterRuntime(module=self.module)
        self._set("PROMPT_WATCH_ADAPTER_RUNTIME", watch_adapter_runtime)
        watch_approval_adapter_runtime = (
            discord_bot_prompt_watch_approval_adapter_runtime.BotPromptWatchApprovalAdapterRuntime(module=self.module)
        )
        self._set("PROMPT_WATCH_APPROVAL_ADAPTER_RUNTIME", watch_approval_adapter_runtime)
        watch_runtime = watch_adapter_runtime.make_prompt_watch_runtime()
        self._set("PROMPT_WATCH_RUNTIME", watch_runtime)
        self._set("_build_steering_watch_relay", cast(Callable[..., object], getattr(watch_runtime, "_build_steering_watch_relay")))
        self._set("_build_approval_followup_relay", cast(Callable[..., object], getattr(watch_runtime, "_build_approval_followup_relay")))
        self._set("mark_busy_steering_handoff", self.mark_busy_steering_handoff)
        self._set("mark_persistent_busy_steering_handoff", self.mark_persistent_busy_steering_handoff)
        self._set("make_steering_watch_relay", watch_runtime.make_steering_watch_relay)
        self._set("run_steering_watch_stream", watch_runtime.run_steering_watch_stream)
        self._set("_steering_watch_channel_typing", watch_runtime.steering_watch_channel_typing)
        self._set("stream_steering_prompt_result_to_channel", watch_runtime.stream_steering_prompt_result_to_channel)
        self._set("make_post_approval_watch_result", watch_runtime.make_post_approval_watch_result)
        self._set("make_approval_followup_relay", watch_runtime.make_approval_followup_relay)
        self._set("stream_post_approval_result_to_channel", watch_approval_adapter_runtime.stream_post_approval_result_to_channel)
        self._set("resolve_approval_followup_channel", watch_approval_adapter_runtime.resolve_approval_followup_channel)
        self._set(
            "stream_post_approval_result_for_interaction",
            watch_approval_adapter_runtime.stream_post_approval_result_for_interaction,
        )
        self._set("run_approval_followup_watch_stream", watch_approval_adapter_runtime.run_approval_followup_watch_stream)

    def mark_busy_steering_handoff(self, target_thread_id: str) -> None:
        self._mark_steering_handoff(target_thread_id)

    def mark_persistent_busy_steering_handoff(self, target_thread_id: str | None) -> None:
        self._mark_steering_handoff(target_thread_id)

    def _mark_steering_handoff(self, target_thread_id: str | None) -> None:
        mark_steering_handoff = cast(Callable[[str | None], float], getattr(self.module, "mark_steering_handoff"))
        _ = mark_steering_handoff(target_thread_id)

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )
