from __future__ import annotations

from dataclasses import dataclass
from types import ModuleType
from typing import cast, TypeAlias
import traceback

import codex_discord_bot_component_deps_runtime as discord_bot_component_deps_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_component_view_deps_runtime as discord_bot_component_view_deps_runtime
import codex_discord_bot_interactive_adapter_runtime as discord_bot_interactive_adapter_runtime
import codex_discord_bot_persistent_busy_component_runtime as discord_bot_persistent_busy_component_runtime
import codex_discord_bot_persistent_component_runtime as discord_bot_persistent_component_runtime
import codex_discord_interaction_component_runtime as discord_interaction_component_runtime
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotComponentWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_interaction_components()
        self._install_component_deps()
        self._install_persistent_components()
        self._install_interactive_runtime()

    def _install_interaction_components(self) -> None:
        interaction_component_runtime = discord_interaction_component_runtime.InteractionComponentRuntime(
            delivery_exceptions=cast(tuple[type[BaseException], ...], getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS")),
            format_exception=traceback.format_exc,
            log=cast(discord_interaction_component_runtime.LogFunc, getattr(self.module, "log_line")),
        )
        self._set("INTERACTION_COMPONENT_RUNTIME", interaction_component_runtime)
        self._set("clear_interaction_message_components", interaction_component_runtime.clear_interaction_message_components)
        self._set("resolve_interaction_channel", interaction_component_runtime.resolve_interaction_channel)
        self._set("require_discord_interaction", self._module_attr("MISC_ADAPTER_RUNTIME", "require_discord_interaction"))
        self._set("require_discord_messageable", self._module_attr("MISC_ADAPTER_RUNTIME", "require_discord_messageable"))
        self._set("require_discord_history_channel", self._module_attr("MISC_ADAPTER_RUNTIME", "require_discord_history_channel"))

    def _install_component_deps(self) -> None:
        component_deps_runtime = discord_bot_component_deps_runtime.BotComponentDepsRuntime(module=self.module)
        component_view_deps_runtime = discord_bot_component_view_deps_runtime.BotComponentViewDepsRuntime(module=self.module)
        self._set("COMPONENT_DEPS_RUNTIME", component_deps_runtime)
        self._set("COMPONENT_VIEW_DEPS_RUNTIME", component_view_deps_runtime)
        self._set("_make_persistent_busy_queue_deps", component_deps_runtime.make_persistent_busy_queue_deps)
        self._set("_make_persistent_busy_steer_action_deps", component_deps_runtime.make_persistent_busy_steer_action_deps)
        self._set("_make_busy_choice_steer_failure_deps", component_deps_runtime.make_busy_choice_steer_failure_deps)
        self._set("_make_busy_choice_steer_result_deps", component_deps_runtime.make_busy_choice_steer_result_deps)
        self._set("_make_busy_choice_steer_action_deps", component_deps_runtime.make_busy_choice_steer_action_deps)
        self._set("_make_busy_choice_queue_action_deps", component_deps_runtime.make_busy_choice_queue_action_deps)
        self._set("_make_busy_choice_stop_action_deps", component_deps_runtime.make_busy_choice_stop_action_deps)
        self._set("_make_busy_choice_view_deps", component_view_deps_runtime.make_busy_choice_view_deps)
        self._set("_make_approval_button_action_deps", component_view_deps_runtime.make_approval_button_action_deps)
        self._set("_make_approval_view_deps", component_view_deps_runtime.make_approval_view_deps)
        self._set("_make_input_choice_button_action_deps", component_view_deps_runtime.make_input_choice_button_action_deps)
        self._set("_make_input_choice_view_deps", component_view_deps_runtime.make_input_choice_view_deps)

    def _install_persistent_components(self) -> None:
        persistent_busy_runtime = discord_bot_persistent_busy_component_runtime.BotPersistentBusyComponentRuntime(
            module=self.module
        )
        persistent_runtime = discord_bot_persistent_component_runtime.BotPersistentComponentRuntime(module=self.module)
        self._set("PERSISTENT_BUSY_COMPONENT_RUNTIME", persistent_busy_runtime)
        self._set("PERSISTENT_COMPONENT_RUNTIME", persistent_runtime)
        self._set("adapt_persistent_interaction", persistent_runtime.adapt_persistent_interaction)
        self._set("require_discord_persistent_interaction", persistent_runtime.require_discord_persistent_interaction)
        self._set("claim_persistent_component_for_persistent_interaction", persistent_runtime.claim_persistent_component_for_persistent_interaction)
        self._set("clear_persistent_interaction_components", persistent_runtime.clear_persistent_interaction_components)
        self._set("clear_busy_interaction_components", persistent_busy_runtime.clear_busy_interaction_components)
        self._set("send_persistent_interaction_response", persistent_runtime.send_persistent_interaction_response)
        self._set("send_busy_interaction_response", persistent_busy_runtime.send_busy_interaction_response)
        self._set("send_persistent_followup_chunks", persistent_runtime.send_persistent_followup_chunks)
        self._set("stream_post_approval_result_for_persistent_interaction", persistent_runtime.stream_post_approval_result_for_persistent_interaction)
        self._set("report_unhandled_component_interaction", persistent_runtime.report_unhandled_component_interaction)
        self._set("handle_persistent_approval_interaction", persistent_runtime.handle_persistent_approval_interaction)
        self._set("handle_persistent_input_choice_interaction", persistent_runtime.handle_persistent_input_choice_interaction)
        self._set("handle_persistent_busy_choice_interaction", persistent_busy_runtime.handle_persistent_busy_choice_interaction)

    def _install_interactive_runtime(self) -> None:
        interactive_adapter_runtime = discord_bot_interactive_adapter_runtime.BotInteractiveAdapterRuntime(
            module=self.module
        )
        interactive_runtime = interactive_adapter_runtime.make_interactive_runtime()
        self._set("INTERACTIVE_ADAPTER_RUNTIME", interactive_adapter_runtime)
        self._set("INTERACTIVE_RUNTIME", interactive_runtime)
        self._set("send_interactive_prompt", interactive_runtime.send_interactive_prompt)
        self._set("submit_interactive_reply", interactive_runtime.submit_interactive_reply)

    def _module_attr(self, module_attr_name: str, attr_name: str) -> ModuleValue:
        module_attr = cast(object, getattr(self.module, module_attr_name))
        return cast(object, getattr(module_attr, attr_name))

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )
