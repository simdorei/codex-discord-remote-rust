from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from functools import partial
from types import ModuleType
from typing import cast, TypeAlias
import sqlite3
import time

import codex_discord_bot_prefix_command_runtime as discord_bot_prefix_command_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_runner_adapter_runtime as discord_bot_runner_adapter_runtime
import codex_discord_bot_session_mirror_adapter_runtime as discord_bot_session_mirror_adapter_runtime
import codex_discord_bot_session_mirror_delegation_runtime as discord_bot_session_mirror_delegation_runtime
import codex_discord_runtime_config as discord_runtime_config
import codex_discord_session_mirror_delegation as discord_session_mirror_delegation
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotSessionRunnerWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_bridge_target_exports()
        self._install_session_mirror_runtime()
        self._install_runner_runtime()
        self._install_context_exhaustion_helpers()
        self._install_session_mirror_delegation()
        self._install_prefix_command_runtime()

    def _install_bridge_target_exports(self) -> None:
        self._set("resolve_target_ref", self._module_attr("BRIDGE_COMMAND_RUNTIME", "resolve_target_ref"))
        self._set("get_interactive_state_for_thread", self._module_attr("BRIDGE_COMMAND_RUNTIME", "get_interactive_state_for_thread"))
        self._set("get_busy_state_for_thread", self._module_attr("BRIDGE_COMMAND_RUNTIME", "get_busy_state_for_thread"))

    def _install_session_mirror_runtime(self) -> None:
        adapter_runtime = discord_bot_session_mirror_adapter_runtime.BotSessionMirrorAdapterRuntime(module=self.module)
        self._set("SESSION_MIRROR_ADAPTER_RUNTIME", adapter_runtime)
        self._set("SESSION_MIRROR_RUNTIME", adapter_runtime.make_session_mirror_runtime())

    def _install_runner_runtime(self) -> None:
        adapter_runtime = discord_bot_runner_adapter_runtime.BotRunnerAdapterRuntime(module=self.module)
        runner_runtime = adapter_runtime.make_runner_runtime()
        self._set("RUNNER_ADAPTER_RUNTIME", adapter_runtime)
        self._set("RUNNER_RUNTIME", runner_runtime)
        self._set("build_runners_message", runner_runtime.build_runners_message)
        self._set("resolve_queue_command_target", runner_runtime.resolve_queue_command_target)
        self._set("retract_queued_ask_for_request", runner_runtime.retract_queued_ask_for_request)
        self._set("codex_app_turn_slot", runner_runtime.codex_app_turn_slot)
        self._set("get_thread_runner", runner_runtime.get_thread_runner)
        self._set("wait_for_codex_thread_idle", runner_runtime.wait_for_codex_thread_idle)
        self._set("enqueue_thread_ask", runner_runtime.enqueue_thread_ask)
        self._set("retract_thread_ask", runner_runtime.retract_thread_ask)
        self._set("report_thread_runner_job_failed", runner_runtime.report_thread_runner_job_failed)
        self._set("thread_runner_loop", runner_runtime.thread_runner_loop)
        self._set("restore_durable_queue_runners", runner_runtime.restore_durable_queue_runners)

    def _install_context_exhaustion_helpers(self) -> None:
        self._set("is_context_exhausted_no_reply_state", discord_session_mirror_delegation.is_context_exhausted_no_reply_state)
        self._set(
            "build_context_exhausted_prompt_message",
            partial(
                discord_session_mirror_delegation.build_context_exhausted_prompt_message,
                format_token_k_func=self._format_token_k,
            ),
        )
        self._set(
            "send_context_exhausted_prompt_notice_if_needed",
            partial(
                discord_session_mirror_delegation.send_context_exhausted_prompt_notice_if_needed,
                bridge_module=cast(discord_session_mirror_delegation.MirrorStatusBridge, getattr(self.module, "BRIDGE_MIRROR_STATUS")),
                send_chunks_func=cast(discord_session_mirror_delegation.SendChunksFunc[object, object], self._send_chunks),
                format_token_k_func=self._format_token_k,
                expected_exceptions=(OSError, RuntimeError, sqlite3.Error),
                log_func=cast(Callable[[str], None], getattr(self.module, "log_line")),
            ),
        )

    def _install_session_mirror_delegation(self) -> None:
        runtime = discord_bot_session_mirror_delegation_runtime.BotSessionMirrorDelegationRuntime(module=self.module)
        self._set("SESSION_MIRROR_DELEGATION_RUNTIME", runtime)
        self._set("should_delegate_output_to_session_mirror", runtime.should_delegate_output_to_session_mirror)
        self._set("should_delegate_session_mirror_output", runtime.should_delegate_session_mirror_output)
        self._set("prepare_session_mirror_delegation", runtime.prepare_session_mirror_delegation)
        self._set("prepare_mapped_session_mirror_output", runtime.prepare_mapped_session_mirror_output)
        self._set("build_prefix_mirror_list", runtime.build_prefix_mirror_list)
        self._set("build_prefix_mirror_check", runtime.build_prefix_mirror_check)
        self._set("prepare_prefix_mapped_session_mirror_output", runtime.prepare_prefix_mapped_session_mirror_output)
        self._set("prepare_prefix_session_mirror_delegation", runtime.prepare_prefix_session_mirror_delegation)

    def _install_prefix_command_runtime(self) -> None:
        host_reboot_allowed_user_ids_configured = bool(discord_runtime_config.get_discord_allowed_user_ids())
        runtime = cast(
            discord_bot_prefix_command_runtime.BotPrefixCommandRuntime[object],
            discord_bot_prefix_command_runtime.BotPrefixCommandRuntime(
                discord_bot_prefix_command_runtime.BotPrefixCommandRuntimeDeps(
                    module=self.module,
                    interactive_state_approval=cast(str, getattr(self.module, "INTERACTIVE_STATE_APPROVAL")),
                    qa_commands_enabled=discord_runtime_config.discord_qa_commands_enabled,
                    host_commands_enabled=discord_runtime_config.discord_host_commands_enabled,
                    host_reboot_allowed_user_ids_configured=lambda: host_reboot_allowed_user_ids_configured,
                    monotonic=time.monotonic,
                )
            )
        )
        self._set("PREFIX_COMMAND_RUNTIME", runtime)
        self._set("_make_prefix_command_deps_factory", runtime.make_prefix_command_deps_factory)
        self._set("_make_prefix_prompt_command_deps", runtime.make_prefix_prompt_command_deps)
        self._set("_make_prefix_mirror_command_deps", runtime.make_prefix_mirror_command_deps)
        self._set("_make_prefix_steer_command_deps", runtime.make_prefix_steer_command_deps)
        self._set("_make_prefix_status_command_deps", runtime.make_prefix_status_command_deps)
        self._set("_make_prefix_queue_command_deps", runtime.make_prefix_queue_command_deps)
        self._set("_make_prefix_resume_command_deps", runtime.make_prefix_resume_command_deps)
        self._set("recover_resident_thread_for_request", runtime.recover_resident_thread_for_request)
        self._set("_make_prefix_archive_command_deps", runtime.make_prefix_archive_command_deps)
        self._set("_make_prefix_approval_command_deps", runtime.make_prefix_approval_command_deps)
        self._set("_make_prefix_qa_command_deps", runtime.make_prefix_qa_command_deps)
        self._set("_make_prefix_new_command_deps", runtime.make_prefix_new_command_deps)
        self._set("_make_prefix_host_command_deps", runtime.make_prefix_host_command_deps)

    async def _send_chunks(self, channel: ModuleValue, text: str, *, context: str) -> ModuleValue:
        send_chunks = cast(Callable[..., Awaitable[object]], getattr(self.module, "send_chunks"))
        return await send_chunks(channel, text, context=context)

    def _format_token_k(self, value: int) -> str:
        bridge = cast(object, getattr(self.module, "BRIDGE_CONTEXT"))
        return cast(Callable[[int], str], getattr(bridge, "format_token_k"))(value)

    def _module_attr(self, module_attr_name: str, attr_name: str) -> ModuleValue:
        module_attr = cast(object, getattr(self.module, module_attr_name))
        return cast(object, getattr(module_attr, attr_name))

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )
