from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from functools import partial
from types import ModuleType
from typing import cast, TypeAlias

import codex_discord_bot_busy_interaction_adapter_runtime as discord_bot_busy_interaction_adapter_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_plain_ask_adapter_runtime as discord_bot_plain_ask_adapter_runtime
import codex_discord_bot_plain_ask_busy_view_runtime as discord_bot_plain_ask_busy_view_runtime
import codex_discord_bot_prompt_delivery_adapter_runtime as discord_bot_prompt_delivery_adapter_runtime
import codex_discord_bot_prompt_transport_adapter_runtime as discord_bot_prompt_transport_adapter_runtime
import codex_discord_bot_view_classes_runtime as discord_bot_view_classes_runtime
import codex_discord_prompt_delivery_snapshot as discord_prompt_delivery_snapshot
from codex_app_server_transport_delivery import BridgeModule
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotPromptFlowWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_prompt_delivery()
        self._install_prompt_transport()
        self._install_busy_interaction()
        self._install_plain_ask()
        self._install_view_classes()

    def _install_prompt_delivery(self) -> None:
        self._set(
            "snapshot_ask_prompt_delivery_state",
            partial(
                discord_prompt_delivery_snapshot.snapshot_ask_prompt_delivery_state,
                deps=discord_prompt_delivery_snapshot.PromptDeliverySnapshotDeps(
                    bridge=cast(BridgeModule, getattr(self.module, "BRIDGE_APP_SERVER_DELIVERY")),
                    log=cast(Callable[[str], None], getattr(self.module, "log_line")),
                ),
            ),
        )
        adapter_runtime = discord_bot_prompt_delivery_adapter_runtime.BotPromptDeliveryAdapterRuntime(module=self.module)
        prompt_runtime = adapter_runtime.make_prompt_delivery_runtime()
        self._set("PROMPT_DELIVERY_ADAPTER_RUNTIME", adapter_runtime)
        self._set("PROMPT_DELIVERY_RUNTIME", prompt_runtime)
        self._set("handle_recorded_busy_transport_prompt", prompt_runtime.handle_recorded_busy_transport_prompt)
        self._set("wait_for_mirrored_busy_delegation_settle", prompt_runtime.wait_for_mirrored_busy_delegation_settle)
        self._set("run_prompt_and_send", prompt_runtime.run_prompt_and_send)
        self._set("send_codex_app_menu_if_available", prompt_runtime.send_codex_app_menu_if_available)

    def _install_prompt_transport(self) -> None:
        adapter_runtime = discord_bot_prompt_transport_adapter_runtime.BotPromptTransportAdapterRuntime(module=self.module)
        transport_runtime = adapter_runtime.make_prompt_transport_runtime()
        self._set("PROMPT_TRANSPORT_ADAPTER_RUNTIME", adapter_runtime)
        self._set("PROMPT_TRANSPORT_RUNTIME", transport_runtime)
        self._set("make_prompt_transport_deps", transport_runtime.make_prompt_transport_deps)
        self._set("make_mapped_prompt_delivery_deps", transport_runtime.make_mapped_prompt_delivery_deps)
        self._set("make_ask_stream_pending_delivery_deps", transport_runtime.make_ask_stream_pending_delivery_deps)
        self._set("_make_discord_ask_relay", transport_runtime.make_discord_ask_relay)
        self._set("_run_ask_stream_in_thread", transport_runtime.run_ask_stream_in_thread)
        self._set("make_ask_stream_busy_result_deps", transport_runtime.make_ask_stream_busy_result_deps)
        self._set("run_ask_stream", transport_runtime.run_ask_stream)

    def _install_busy_interaction(self) -> None:
        adapter_runtime = discord_bot_busy_interaction_adapter_runtime.BotBusyInteractionAdapterRuntime(module=self.module)
        busy_runtime = adapter_runtime.make_busy_interaction_runtime()
        self._set("BUSY_INTERACTION_ADAPTER_RUNTIME", adapter_runtime)
        self._set("BUSY_INTERACTION_RUNTIME", busy_runtime)
        self._set("send_busy_direct_followup", busy_runtime.send_busy_direct_followup)
        self._set("send_busy_stale_block_message", busy_runtime.send_busy_stale_block_message)
        self._set("send_busy_codex_app_menu_if_available", busy_runtime.send_busy_codex_app_menu_if_available)
        self._set("send_persistent_busy_steering_start_ack", busy_runtime.send_persistent_busy_steering_start_ack)
        self._set("build_codex_app_steering_not_accepted_message", adapter_runtime.build_codex_app_steering_not_accepted_message)
        self._set("build_busy_choice_message", adapter_runtime.build_busy_choice_message)

    def _install_plain_ask(self) -> None:
        busy_view_runtime = discord_bot_plain_ask_busy_view_runtime.BotPlainAskBusyViewRuntime(module=self.module)
        adapter_runtime = discord_bot_plain_ask_adapter_runtime.BotPlainAskAdapterRuntime(module=self.module)
        plain_runtime = adapter_runtime.make_plain_ask_runtime()
        self._set("PLAIN_ASK_BUSY_VIEW_RUNTIME", busy_view_runtime)
        self._set("PLAIN_ASK_ADAPTER_RUNTIME", adapter_runtime)
        self._set("PLAIN_ASK_RUNTIME", plain_runtime)
        self._set("run_prompt_flow", plain_runtime.run_prompt_flow)
        self._set("make_busy_choice_payload", plain_runtime.make_busy_choice_payload)
        self._set("send_busy_choice_message", plain_runtime.send_busy_choice_message)
        self._set("enqueue_plain_thread_ask", plain_runtime.enqueue_plain_thread_ask)
        self._set("send_plain_busy_choice_message", plain_runtime.send_plain_busy_choice_message)
        self._set("send_plain_ask_chunks", plain_runtime.send_plain_ask_chunks)
        self._set("make_busy_choice_view", busy_view_runtime.make_busy_choice_view)
        self._set("handle_busy_plain_ask", plain_runtime.handle_busy_plain_ask)
        self._set("handle_plain_ask", plain_runtime.handle_plain_ask)

    def _install_view_classes(self) -> None:
        runtime = discord_bot_view_classes_runtime.BotViewClassesRuntime(module=self.module)
        input_choice_button, input_choice_view = runtime.make_input_choice_classes()
        self._set("VIEW_CLASSES_RUNTIME", runtime)
        self._set("ApprovalView", runtime.make_approval_view_class())
        self._set("InputChoiceButton", input_choice_button)
        self._set("InputChoiceView", input_choice_view)
        self._set("BusyChoiceView", runtime.make_busy_choice_view_class())

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )
