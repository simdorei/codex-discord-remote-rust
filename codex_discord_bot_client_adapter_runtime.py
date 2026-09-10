from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import importlib
from collections.abc import Callable, Coroutine, Mapping
from dataclasses import dataclass
from types import ModuleType
from typing import final, cast, TypeAlias

import codex_discord_channel_cache as discord_channel_cache
import codex_discord_channel_gate as discord_channel_gate
import codex_discord_delivery as discord_delivery
import codex_discord_delivery_state as discord_delivery_state
import codex_discord_interaction_log as discord_interaction_log
import codex_discord_interaction_gate as discord_interaction_gate
import codex_discord_message_gate as discord_message_gate
import codex_discord_ready_cleanup as discord_ready_cleanup
import codex_discord_runtime_config as discord_runtime_config
import codex_discord_startup_probe as discord_startup_probe
from codex_discord_bot_client_adapter_base import BotClientAdapterBase
from codex_discord_text import parse_bounded_float_env
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotClientAdapterRuntime(BotClientAdapterBase):
    module: ModuleType

    def make_codex_discord_bot_class(self, logging_command_tree_class: type[ModuleValue]) -> type[ModuleValue]:
        runtime = self
        discord_module = importlib.import_module("discord")
        client_base = cast(type[object], getattr(discord_module, "Client"))
        intents_type = cast(object, getattr(discord_module, "Intents"))
        interaction_type_enum = cast(object, getattr(discord_module, "InteractionType"))

        @final
        class CodexDiscordBot(client_base):
            def __init__(
                self,
                *,
                allowed_channel_ids: set[int],
                allowed_user_ids: set[int],
                startup_channel_id: int | None,
                guild_id: int | None,
                enable_prefix_commands: bool,
                plain_ask_mention_user_ids: set[int] | None = None,
            ) -> None:
                intents = cast(Callable[[], object], getattr(intents_type, "default"))()
                setattr(intents, "message_content", enable_prefix_commands)
                cast(Callable[..., None], getattr(client_base, "__init__"))(
                    self,
                    intents=intents,
                    enable_debug_events=True,
                )
                self.tree = cast(Callable[[object], object], logging_command_tree_class)(self)
                self.allowed_channel_ids = allowed_channel_ids
                self.allowed_user_ids = allowed_user_ids
                self.startup_channel_id = startup_channel_id
                self.guild_id = guild_id
                self.enable_prefix_commands = enable_prefix_commands
                self.plain_ask_mention_user_ids = set(plain_ask_mention_user_ids or set())
                self.history_poll_seconds = discord_runtime_config.get_history_poll_seconds(default=cast(float, getattr(runtime.module, "HISTORY_POLL_DEFAULT_SECONDS")))
                self.session_mirror_poll_seconds = parse_bounded_float_env(
                    "DISCORD_SESSION_MIRROR_POLL_SECONDS",
                    default=cast(float, getattr(runtime.module, "SESSION_MIRROR_POLL_DEFAULT_SECONDS")),
                    minimum=0.25,
                    maximum=60.0,
                )
                self._history_poll_task: ModuleValue | None = None
                self._stop_marker_task: ModuleValue | None = None
                self._history_poll_primed_channels: set[int] = set()
                self._history_poll_last_at = "-"
                self._session_mirror_task: ModuleValue | None = None
                self._session_mirror_last_at = "-"
                self._session_mirror_seen_agent_messages: dict[str, dict[str, float]] = {}
                self._session_mirror_seen_user_messages: dict[str, dict[str, float]] = {}
                self._session_mirror_archive_skip_logged: set[str] = set()
                self._processed_message_ids: dict[str, float] = {}
                self._logged_socket_event_ids: dict[str, float] = {}
                self._slash_sync_last_at = "-"
                self._slash_sync_status = "-"
                self._slash_sync_commands = "-"

            def is_allowed_channel(self, channel_id: int | None) -> bool:
                if not self.allowed_channel_ids:
                    return True
                return channel_id in self.allowed_channel_ids

            def is_allowed_message_channel(self, channel: discord_channel_gate.MessageChannelLike) -> bool:
                return discord_channel_gate.is_allowed_message_channel(
                    channel, is_allowed_channel_func=self.is_allowed_channel, is_mirrored_channel_id_func=cast(Callable[[object], bool], runtime._module_func("is_mirrored_channel_id")),
                )

            def is_allowed_user(self, user_id: int | None) -> bool:
                if self.allowed_user_ids:
                    return user_id in self.allowed_user_ids
                return discord_interaction_gate.is_discord_user_allowed(user_id)

            async def setup_hook(self) -> None:
                await runtime._await_runtime("LIFECYCLE_RUNTIME", "setup_hook", self)

            def get_cached_channel_or_thread(self, channel_id: int) -> tuple[ModuleValue | None, str]:
                return discord_channel_cache.get_cached_channel_or_thread(
                    cast(discord_channel_cache.ClientChannelCache[object], cast(object, self)),
                    channel_id,
                )

            async def probe_channel_access(self, label: str, channel_id: int) -> None:
                await discord_startup_probe.probe_channel_access(label, channel_id, deps=cast(discord_startup_probe.StartupProbeDeps[object], runtime._module_func("_make_startup_probe_deps")(self)))

            async def cleanup_stale_busy_choice_components(self) -> None:
                await discord_ready_cleanup.cleanup_stale_busy_choice_components(deps=cast(discord_ready_cleanup.StaleBusyChoiceCleanupDeps[object], runtime._module_func("_make_stale_busy_choice_cleanup_deps")(self)))

            async def log_startup_diagnostics(self) -> None:
                await discord_startup_probe.log_startup_diagnostics(cast(discord_startup_probe.StartupDiagnosticsDeps, runtime._module_func("_make_startup_diagnostics_deps")(self, self.probe_channel_access)))

            async def start_history_polling(self) -> None:
                await runtime._await_runtime("HISTORY_RUNTIME", "start_history_polling", self)

            async def history_poll_loop(self) -> None:
                await runtime._await_runtime("HISTORY_RUNTIME", "history_poll_loop", self)

            async def start_stop_marker_watcher(self) -> None:
                await runtime._await_runtime("STOP_RUNTIME", "start_stop_marker_watcher", self)

            async def stop_marker_loop(self) -> None:
                await runtime._await_runtime("STOP_RUNTIME", "stop_marker_loop", self)

            async def poll_history_channel(self, label: str, channel_id: int) -> None:
                await runtime._await_runtime("HISTORY_RUNTIME", "poll_history_channel", self, label, channel_id)

            async def process_history_poll_message(self, message: ModuleValue, channel_id: int) -> None:
                await runtime._await_runtime("HISTORY_RUNTIME", "process_history_poll_message", self, message, channel_id)

            async def start_session_mirroring(self) -> None:
                await runtime._await_runtime("SESSION_MIRROR_RUNTIME", "start_session_mirroring", self)

            async def session_mirror_loop(self) -> None:
                await runtime._await_runtime("SESSION_MIRROR_RUNTIME", "session_mirror_loop", self)

            async def resolve_session_mirror_channel(self, discord_thread_id: int) -> ModuleValue | None:
                return await runtime._await_runtime_value("SESSION_MIRROR_RUNTIME", "resolve_session_mirror_channel", self, discord_thread_id)

            def get_session_mirror_seen_agent_messages(self, codex_thread_id: str) -> dict[str, float]:
                return self._session_mirror_seen_agent_messages.setdefault(codex_thread_id, {})

            def get_session_mirror_seen_user_messages(self, codex_thread_id: str) -> dict[str, float]:
                return self._session_mirror_seen_user_messages.setdefault(codex_thread_id, {})

            async def send_session_mirror_item(
                self,
                channel: ModuleValue,
                item: ModuleValue,
                *,
                target_thread_id: str,
                target_ref: str,
            ) -> None:
                await runtime._await_runtime(
                    "SESSION_MIRROR_RUNTIME",
                    "send_session_mirror_item",
                    channel,
                    item,
                    target_thread_id=target_thread_id,
                    target_ref=target_ref,
                )

            async def mirror_session_target(self, target: Mapping[str, ModuleValue]) -> None:
                await runtime._await_runtime("SESSION_MIRROR_RUNTIME", "mirror_session_target", self, target)

            async def on_ready(self) -> None:
                await runtime._await_runtime("LIFECYCLE_RUNTIME", "on_ready", self)

            async def on_interaction(self, interaction: ModuleValue) -> None:
                interaction_type = discord_interaction_log.format_interaction_type(
                    cast(discord_interaction_log.InteractionTypeLike, interaction)
                )
                command_name = discord_delivery.get_interaction_command_name(
                    cast(discord_delivery_state.InteractionCommandSource, interaction)
                )
                custom_id = discord_interaction_log.get_interaction_custom_id(
                    cast(discord_interaction_log.InteractionDataLike, interaction)
                )
                runtime._log(
                    "interaction_received "
                    + f"type={interaction_type} command={command_name} "
                    + f"custom_id={custom_id} channel={getattr(interaction, 'channel_id', None)} "
                    + f"user={getattr(getattr(interaction, 'user', None), 'id', '-')}"
                )
                if getattr(interaction, "type", None) == getattr(interaction_type_enum, "component", None):
                    _ = asyncio.create_task(cast(Coroutine[object, object, object], runtime._module_func("report_unhandled_component_interaction")(interaction)))

            async def on_socket_raw_receive(self, message: str | bytes) -> None:
                await runtime._await_runtime("SOCKET_RUNTIME", "on_socket_raw_receive", self, message)

            async def on_socket_response(self, payload: ModuleValue) -> None:
                await runtime._await_runtime("SOCKET_RUNTIME", "on_socket_response", self, payload)

            def is_tracked_socket_message_channel(self, channel_id: int | None) -> tuple[bool, str]:
                return cast(
                    tuple[bool, str],
                    runtime._runtime_func("SOCKET_RUNTIME", "is_tracked_socket_message_channel")(self, channel_id),
                )

            async def log_socket_payload(self, payload: ModuleValue) -> None:
                await runtime._await_runtime("SOCKET_RUNTIME", "log_socket_payload", self, payload)

            def format_socket_interaction_user(self, data: ModuleValue) -> str:
                return str(runtime._runtime_func("SOCKET_RUNTIME", "format_socket_interaction_user")(data))

            async def on_message(self, message: ModuleValue) -> None:
                def claim_message(message: ModuleValue) -> bool:
                    return bool(runtime._module_func("claim_discord_message")(self, message))

                def get_message_id(message: ModuleValue) -> discord_message_gate.DiscordIdValue:
                    return cast(discord_message_gate.DiscordIdValue, runtime._module_func("get_discord_message_id")(message))

                async def process_message(message: ModuleValue, *, source: str) -> None:
                    await runtime._await_runtime("MESSAGE_RUNTIME", "process_discord_message", self, message, source=source)

                def mark_processed(message: ModuleValue) -> None:
                    _ = runtime._module_func("mark_discord_message_processed")(self, message)

                await discord_message_gate.process_gateway_message(
                    message,
                    deps=discord_message_gate.GatewayMessageDeps(
                        discord_client=cast(discord_message_gate.DiscordClientWithMentions, cast(object, self)),
                        claim_message=claim_message,
                        get_message_id=get_message_id,
                        process_message=process_message,
                        mark_processed=mark_processed,
                        log=runtime._log,
                    ),
                )

            async def process_discord_message(self, message: ModuleValue, *, source: str) -> None:
                await runtime._await_runtime("MESSAGE_RUNTIME", "process_discord_message", self, message, source=source)

        return CodexDiscordBot
