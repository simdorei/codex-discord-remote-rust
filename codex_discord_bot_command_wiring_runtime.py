from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast, TypeAlias

import codex_discord_bot_button_qa_adapter_runtime as discord_bot_button_qa_adapter_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_interaction_delivery_runtime as discord_bot_interaction_delivery_runtime
import codex_discord_bot_new_thread_adapter_runtime as discord_bot_new_thread_adapter_runtime
import codex_discord_bot_prefix_adapter_runtime as discord_bot_prefix_adapter_runtime
import codex_discord_bot_skill_slash_adapter_runtime as discord_bot_skill_slash_adapter_runtime
import codex_discord_bot_steering_ack_runtime as discord_bot_steering_ack_runtime
import codex_discord_new_thread_flow as discord_new_thread_flow
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotCommandWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_interaction_delivery()
        skill_slash_runtime = self._install_skill_slash()
        self._install_steering_ack()
        self._install_button_qa()
        self._install_new_thread_and_prefix(skill_slash_runtime)

    def _install_interaction_delivery(self) -> None:
        runtime = discord_bot_interaction_delivery_runtime.BotInteractionDeliveryRuntime(module=self.module)
        self._set("INTERACTION_DELIVERY_RUNTIME", runtime)
        self._set("run_bridge_and_send", runtime.run_bridge_and_send)
        self._set("send_interaction_chunks", runtime.send_interaction_chunks)
        self._set("send_followup_chunks", runtime.send_followup_chunks)
        self._set("send_busy_followup_chunks", runtime.send_busy_followup_chunks)
        self._set("send_direct_followup", runtime.send_direct_followup)
        self._set("send_skill_slash_direct_followup", runtime.send_skill_slash_direct_followup)
        self._set("run_interaction_bridge_and_send", runtime.run_interaction_bridge_and_send)

    def _install_skill_slash(self) -> discord_bot_skill_slash_adapter_runtime.BotSkillSlashAdapterRuntime:
        runtime = discord_bot_skill_slash_adapter_runtime.BotSkillSlashAdapterRuntime(module=self.module)
        self._set("SKILL_SLASH_ADAPTER_RUNTIME", runtime)
        self._set("handle_skill_slash_plain_ask", runtime.handle_skill_slash_plain_ask)
        self._set("get_skill_slash_interaction_command_name", runtime.get_skill_slash_interaction_command_name)
        self._set("make_skill_slash_source_message", runtime.make_skill_slash_source_message)
        self._set("handle_slash_ask", runtime.handle_slash_ask)
        self._set("send_skill_slash_interaction_chunks", runtime.send_skill_slash_interaction_chunks)
        self._set("SKILL_SLASH_RUNTIME", runtime.make_skill_slash_runtime())
        self._set("handle_slash_interview", runtime.handle_slash_interview)
        return runtime

    def _install_steering_ack(self) -> None:
        build_message = cast(Callable[[str], str], getattr(self.module, "build_steering_start_message_text"))
        self._set("build_steering_start_message", build_message)
        runtime = discord_bot_steering_ack_runtime.BotSteeringAckRuntime(
            send_message_tracked=cast(discord_bot_steering_ack_runtime.SteeringAckSender, getattr(self.module, "send_message_tracked")),
            build_steering_start_message=build_message,
            delivery_exceptions=cast(tuple[type[BaseException], ...], getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS")),
            log=cast(Callable[[str], None], getattr(self.module, "log_line")),
            format_log_text_len=cast(Callable[[str], int | str], getattr(self.module, "format_log_text_len")),
        )
        self._set("STEERING_ACK_RUNTIME", runtime)
        self._set("send_steering_start_ack", runtime.send_steering_start_ack)

    def _install_button_qa(self) -> None:
        runtime = discord_bot_button_qa_adapter_runtime.BotButtonQaAdapterRuntime(module=self.module)
        self._set("BUTTON_QA_ADAPTER_RUNTIME", runtime)
        self._set("RuntimeSlashBotTypeError", discord_bot_button_qa_adapter_runtime.RuntimeSlashBotTypeError)
        self._set("send_busy_choice_qa_message_tracked", runtime.send_busy_choice_qa_message_tracked)
        self._set("send_persistent_button_qa_message_tracked", runtime.send_persistent_button_qa_message_tracked)
        self._set("make_button_qa_busy_choice_payload", runtime.make_button_qa_busy_choice_payload)
        self._set("handle_lifecycle_qa_busy_choice_interaction", runtime.handle_lifecycle_qa_busy_choice_interaction)
        self._set("handle_steer_qa_busy_choice_interaction", runtime.handle_steer_qa_busy_choice_interaction)
        self._set("make_persistent_qa_approval_view", runtime.make_persistent_qa_approval_view)
        self._set("make_persistent_qa_input_choice_view", runtime.make_persistent_qa_input_choice_view)
        self._set("handle_persistent_qa_approval_interaction", runtime.handle_persistent_qa_approval_interaction)
        self._set("handle_persistent_qa_input_choice_interaction", runtime.handle_persistent_qa_input_choice_interaction)
        self._set("require_runtime_codex_bot", runtime.require_runtime_codex_bot)
        self._set("BUTTON_QA_RUNTIME", runtime.make_button_qa_runtime())
        self._set("run_discord_button_qa", runtime.run_discord_button_qa)
        self._set("run_prefix_discord_button_qa", runtime.run_prefix_discord_button_qa)
        self._set("run_runtime_discord_button_qa", runtime.run_runtime_discord_button_qa)

    def _install_new_thread_and_prefix(
        self,
        skill_slash_runtime: discord_bot_skill_slash_adapter_runtime.BotSkillSlashAdapterRuntime,
    ) -> None:
        self._set("build_runtime_discord_doctor_message", self._module_attr("MISC_ADAPTER_RUNTIME", "build_runtime_discord_doctor_message"))
        self._set("refresh_runtime_discord_bridge_session", self._module_attr("MISC_ADAPTER_RUNTIME", "refresh_runtime_discord_bridge_session"))
        self._set("format_discord_new_thread_prefix", discord_new_thread_flow.format_discord_new_thread_prefix)
        new_thread_runtime = discord_bot_new_thread_adapter_runtime.BotNewThreadAdapterRuntime(module=self.module)
        self._set("NEW_THREAD_ADAPTER_RUNTIME", new_thread_runtime)
        self._set("run_discord_new_thread", new_thread_runtime.run_discord_new_thread)
        self._set("handle_slash_new", new_thread_runtime.handle_slash_new)
        prefix_runtime = discord_bot_prefix_adapter_runtime.BotPrefixAdapterRuntime(module=self.module)
        self._set("PREFIX_ADAPTER_RUNTIME", prefix_runtime)
        self._set("send_prefix_chunks", prefix_runtime.send_prefix_chunks)
        self._set("handle_prefix_plain_ask", prefix_runtime.handle_prefix_plain_ask)
        self._set("stream_prefix_steering_prompt_result_to_channel", prefix_runtime.stream_prefix_steering_prompt_result_to_channel)
        self._set("refresh_prefix_mirror_bridge_session", prefix_runtime.refresh_prefix_mirror_bridge_session)
        self._set("sync_prefix_mirror_codex", prefix_runtime.sync_prefix_mirror_codex)
        self._set("send_prefix_approval_interactive_prompt", prefix_runtime.send_prefix_approval_interactive_prompt)
        self._set("run_prefix_discord_new_thread", new_thread_runtime.run_prefix_discord_new_thread)
        self._set("build_system_resources_message", prefix_runtime.build_system_resources_message)
        self._set("_make_prefix_dispatch_deps", prefix_runtime.make_prefix_dispatch_deps)
        self._set("send_skill_slash_interaction_chunks", skill_slash_runtime.send_skill_slash_interaction_chunks)

    def _module_attr(self, module_attr_name: str, attr_name: str) -> ModuleValue:
        module_attr = cast(object, getattr(self.module, module_attr_name))
        return cast(object, getattr(module_attr, attr_name))

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )
