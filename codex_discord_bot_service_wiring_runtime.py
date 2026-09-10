from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from functools import partial
from pathlib import Path
from types import ModuleType
from typing import TypeAlias, cast

import codex_discord_attachments as discord_attachments
import codex_discord_bot_archive_mirror_cleanup_runtime as discord_bot_archive_mirror_cleanup_runtime
import codex_discord_bot_channel_typing_runtime as discord_bot_channel_typing_runtime
import codex_discord_bot_client_adapter_runtime as discord_bot_client_adapter_runtime
import codex_discord_bot_component_wiring_runtime as discord_bot_component_wiring_runtime
import codex_discord_bot_delivery_adapter_runtime as discord_bot_delivery_adapter_runtime
import codex_discord_bot_misc_adapter_runtime as discord_bot_misc_adapter_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_ready_adapter_runtime as discord_bot_ready_adapter_runtime
import codex_discord_bot_session_context_adapter_runtime as discord_bot_session_context_adapter_runtime
import codex_discord_bot_socket_runtime as discord_bot_socket_runtime
import codex_discord_bot_stop_runtime as discord_bot_stop_runtime
import codex_discord_channel_typing as discord_channel_typing
import codex_discord_empty_content_notice as discord_empty_content_notice
import codex_discord_message_payload_runtime as discord_message_payload_runtime
import codex_discord_session_mirror_archive as discord_session_mirror_archive
import codex_discord_stop_marker as discord_stop_marker
from codex_remote_mcp_binding import prepare_remote_mcp_restart_handoff
from codex_discord_text import build_startup_notice as build_startup_notice_text

ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotServiceWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._set("traceback", traceback)
        self._install_misc_adapter()
        self._install_ready_runtime()
        self._install_socket_runtime()
        self._install_stop_runtime()
        self._install_client_runtime()
        self._install_delivery_runtime()
        self._set("build_startup_notice", build_startup_notice_text)
        self._install_channel_typing_runtime()
        self._set("stream_recorded_busy_steering_result", self._module_attr("MISC_ADAPTER_RUNTIME", "stream_recorded_busy_steering_result"))
        self._install_message_payload_runtime()
        self._install_session_context_runtime()
        self._install_component_wiring_runtime()
        self._install_archive_mirror_cleanup_runtime()

    def _install_misc_adapter(self) -> None:
        runtime = discord_bot_misc_adapter_runtime.BotMiscAdapterRuntime(module=self.module)
        self._set("MISC_ADAPTER_RUNTIME", runtime)
        self._set("LoggingCommandTree", runtime.make_logging_command_tree_class())

    def _install_ready_runtime(self) -> None:
        adapter_runtime = discord_bot_ready_adapter_runtime.BotReadyAdapterRuntime(module=self.module)
        ready_runtime = adapter_runtime.make_ready_runtime()
        self._set("READY_ADAPTER_RUNTIME", adapter_runtime)
        self._set("READY_RUNTIME", ready_runtime)
        self._set("_make_plain_ask_message_content_deps", ready_runtime.make_plain_ask_message_content_deps)
        self._set("_make_startup_probe_deps", ready_runtime.make_startup_probe_deps)
        self._set("_make_stale_busy_choice_cleanup_deps", ready_runtime.make_stale_busy_choice_cleanup_deps)
        self._set("_make_ready_maintenance_deps", ready_runtime.make_ready_maintenance_deps)
        self._set("_make_startup_diagnostics_deps", ready_runtime.make_startup_diagnostics_deps)

    def _install_socket_runtime(self) -> None:
        runtime = discord_bot_socket_runtime.BotSocketRuntime(
            discord_bot_socket_runtime.BotSocketRuntimeDeps(
                socket_event_log_id_limit=cast(int, getattr(self.module, "SOCKET_EVENT_LOG_ID_LIMIT")),
                is_mirrored_channel_id=self._is_mirrored_channel_id,
                delivery_exceptions=cast(
                    tuple[type[BaseException], ...],
                    getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS"),
                ),
                log=self._log,
            )
        )
        self._set("SOCKET_RUNTIME", runtime)

    def _install_stop_runtime(self) -> None:
        runtime = discord_bot_stop_runtime.BotStopRuntime(
            discord_bot_stop_runtime.BotStopRuntimeDeps(
                get_stop_request_path=self._stop_request_path,
                get_poll_seconds=lambda: cast(float, getattr(self.module, "STOP_MARKER_POLL_SECONDS")),
                get_drain_timeout_seconds=lambda: cast(
                    float,
                    getattr(self.module, "STOP_MARKER_DRAIN_TIMEOUT_SECONDS"),
                ),
                get_close_timeout_seconds=lambda: cast(
                    float,
                    getattr(self.module, "STOP_MARKER_CLOSE_TIMEOUT_SECONDS"),
                ),
                create_task=lambda coro: asyncio.create_task(coro),
                wait_for=lambda awaitable, timeout: asyncio.wait_for(awaitable, timeout=timeout),
                sleep=lambda seconds: asyncio.sleep(seconds),
                set_delivery_stopping=self._set_delivery_stopping,
                wait_for_delivery_drain=self._wait_for_delivery_drain,
                exit_bot_process=self._exit_bot_process,
                log=self._log,
                prepare_restart_handoff=lambda: prepare_remote_mcp_restart_handoff(
                    self._log
                ),
            )
        )
        self._set("STOP_RUNTIME", runtime)

    def _install_client_runtime(self) -> None:
        adapter_runtime = discord_bot_client_adapter_runtime.BotClientAdapterRuntime(module=self.module)
        logging_command_tree = cast(type[ModuleValue], getattr(self.module, "LoggingCommandTree"))
        self._set("CLIENT_ADAPTER_RUNTIME", adapter_runtime)
        self._set("CodexDiscordBot", adapter_runtime.make_codex_discord_bot_class(logging_command_tree))

    def _install_delivery_runtime(self) -> None:
        adapter_runtime = discord_bot_delivery_adapter_runtime.BotDeliveryAdapterRuntime(module=self.module)
        runtime = adapter_runtime.make_delivery_runtime()
        self._set("DISCORD_DELIVERY_ADAPTER_RUNTIME", adapter_runtime)
        self._set("DISCORD_DELIVERY_RUNTIME", runtime)
        self._set("sync_discord_delivery_legacy_config", runtime.sync_legacy_config)
        self._set("set_discord_delivery_stopping", runtime.set_stopping)
        self._set("clear_discord_delivery_stopping", runtime.clear_stopping)
        self._set("is_discord_delivery_stopping", runtime.is_stopping)
        self._set("begin_discord_delivery", runtime.begin)
        self._set("end_discord_delivery", runtime.end)
        self._set("wait_for_discord_delivery_drain", runtime.wait_for_drain)
        self._set("split_delivery_chunks", runtime.split_chunks)
        self._set("send_chunks", runtime.send_chunks)
        self._set("send_session_mirror_attachment", runtime.send_attachment)
        self._set("send_discord_restarting_notice", runtime.send_restarting_notice)
        self._set("send_message_tracked", runtime.send_message_tracked)
        self._set("send_interaction_response_tracked", runtime.send_interaction_response_tracked)
        self._set("send_interaction_not_allowed", runtime.send_interaction_not_allowed)

    def _install_channel_typing_runtime(self) -> None:
        channel_typing = cast(
            discord_bot_channel_typing_runtime.MessageableChannelTyping,
            cast(ModuleValue, partial(discord_channel_typing.channel_typing, log_func=self._log)),
        )
        runtime = discord_bot_channel_typing_runtime.BotChannelTypingRuntime(
            channel_typing_factory=lambda: channel_typing,
            messageable_channel_resolver=lambda: cast(
                discord_bot_channel_typing_runtime.MessageableChannelResolver,
                self._module_func("require_discord_messageable_channel"),
            ),
            steering_handoff_marker=lambda: cast(
                discord_bot_channel_typing_runtime.SteeringHandoffMarker,
                self._module_func("mark_steering_handoff"),
            ),
        )
        self._set("channel_typing", channel_typing)
        self._set("CHANNEL_TYPING_RUNTIME", runtime)
        self._set("mapped_prompt_delivery_channel_typing", runtime.mapped_prompt_delivery_channel_typing)
        self._set("prompt_delivery_channel_typing", runtime.prompt_delivery_channel_typing)
        self._set("approval_followup_channel_typing", runtime.approval_followup_channel_typing)
        self._set("prefix_steer_channel_typing", runtime.prefix_steer_channel_typing)
        self._set("mark_optional_steering_handoff", runtime.mark_optional_steering_handoff)

    def _install_message_payload_runtime(self) -> None:
        runtime = discord_message_payload_runtime.MessagePayloadRuntime(
            attachment_download_dir=cast(Path, getattr(self.module, "ATTACHMENT_DOWNLOAD_DIR")),
            get_message_id=cast(
                discord_message_payload_runtime.MessageIdFunc,
                self._module_func("get_discord_message_id"),
            ),
            message_has_non_text_payload=cast(
                discord_empty_content_notice.MessageHasNonTextPayloadFunc,
                discord_attachments.message_has_non_text_payload,
            ),
            send_chunks=cast(discord_empty_content_notice.SendChunksFunc, self._module_func("send_chunks")),
            log=self._log,
        )
        self._set("MESSAGE_PAYLOAD_RUNTIME", runtime)
        self._set("build_prompt_with_discord_attachments", runtime.build_prompt_with_discord_attachments)
        self._set("maybe_send_empty_content_notice", runtime.maybe_send_empty_content_notice)

    def _install_session_context_runtime(self) -> None:
        adapter_runtime = discord_bot_session_context_adapter_runtime.BotSessionContextAdapterRuntime(module=self.module)
        runtime = adapter_runtime.make_session_context_runtime()
        self._set("SESSION_CONTEXT_ADAPTER_RUNTIME", adapter_runtime)
        self._set("SESSION_CONTEXT_RUNTIME", runtime)
        self._set("cleanup_recent_discord_origin_prompts", runtime.cleanup_recent_discord_origin_prompts)
        self._set("mark_recent_discord_origin_prompt", runtime.mark_recent_discord_origin_prompt)
        self._set("should_skip_discord_origin_prompt", runtime.should_skip_discord_origin_prompt)
        self._set("build_interactive_notice_from_payload", runtime.build_interactive_notice_from_payload)
        self._set("extract_message_text_from_payload", runtime.extract_message_text_from_payload)
        self._set("extract_user_text_from_session_event", runtime.extract_user_text_from_session_event)
        self._set("iter_recent_session_tail_events", runtime.iter_recent_session_tail_events)
        self._set("clamp_context_refresh_limit", runtime.clamp_context_refresh_limit)
        self._set("collect_context_refresh_items", runtime.collect_context_refresh_items)
        self._set("format_context_refresh_item", runtime.format_context_refresh_item)
        self._set("has_recent_codex_app_user_prompt", runtime.has_recent_codex_app_user_prompt)
        self._set("make_session_mirror_item", runtime.make_session_mirror_item)
        self._set("collect_session_mirror_items", runtime.collect_session_mirror_items)

    def _install_component_wiring_runtime(self) -> None:
        runtime = discord_bot_component_wiring_runtime.BotComponentWiringRuntime(module=self.module)
        self._set("COMPONENT_WIRING_RUNTIME", runtime)
        runtime.install()

    def _install_archive_mirror_cleanup_runtime(self) -> None:
        runtime = discord_bot_archive_mirror_cleanup_runtime.BotArchiveMirrorCleanupRuntime(
            discord_bot_archive_mirror_cleanup_runtime.BotArchiveMirrorCleanupRuntimeDeps(
                get_mirror_db_path=lambda: cast(Path, getattr(self.module, "MIRROR_DB_PATH")),
                get_session_mirror_state=cast(
                    Callable[[], discord_session_mirror_archive.SessionMirrorStateLike],
                    self._module_func("get_session_mirror_state"),
                ),
                release_session_mirror_output_target=cast(
                    Callable[[str | None], Awaitable[bool]],
                    self._module_func("release_session_mirror_output_target"),
                ),
                format_log_argv=cast(Callable[[list[str]], str], self._module_func("format_log_argv")),
                log=self._log,
            )
        )
        self._set("ARCHIVE_MIRROR_CLEANUP_RUNTIME", runtime)
        self._set("cleanup_archived_session_mirror_state", runtime.cleanup_archived_session_mirror_state)
        self._set("_archive_mirror_cleanup_deps", runtime.archive_mirror_cleanup_deps)
        self._set("cleanup_archive_mirror_after_bridge_command", runtime.cleanup_archive_mirror_after_bridge_command)

    def _set_delivery_stopping(self, reason: str) -> None:
        cast(discord_stop_marker.SetDeliveryStopping, self._module_func("set_discord_delivery_stopping"))(reason)

    def _wait_for_delivery_drain(self, *, timeout_seconds: float, reason: str) -> Awaitable[bool]:
        return cast(discord_stop_marker.WaitForDeliveryDrain, self._module_func("wait_for_discord_delivery_drain"))(timeout_seconds=timeout_seconds, reason=reason)

    def _exit_bot_process(self, exit_code: int, *, reason: str) -> None:
        cast(discord_stop_marker.ExitBotProcess, self._module_func("exit_bot_process"))(exit_code, reason=reason)

    def _stop_request_path(self) -> Path:
        return cast(Path, getattr(self.module, "STOP_REQUEST_PATH"))

    def _is_mirrored_channel_id(self, channel_id: int | None) -> bool:
        return cast(Callable[[int | None], bool], self._module_func("is_mirrored_channel_id"))(channel_id)

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
