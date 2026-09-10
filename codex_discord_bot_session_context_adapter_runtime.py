from __future__ import annotations

from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import cast, TypeAlias

import codex_discord_bridge_protocols as discord_bridge_protocols
import codex_app_server_transport as app_server_transport
import codex_discord_runtime as discord_runtime
import codex_discord_session_context_runtime as discord_session_context_runtime
import codex_discord_session_mirror_detail_runtime as discord_session_mirror_detail_runtime
from codex_session_events import JsonEvent, JsonValue
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotSessionContextAdapterRuntime:
    module: ModuleType

    def make_session_context_runtime(self) -> discord_session_context_runtime.SessionContextRuntime:
        detail_runtime = discord_session_mirror_detail_runtime.SessionMirrorDetailRuntime(
            get_db_path=lambda: cast(Path, getattr(self.module, "MIRROR_DB_PATH"))
        )
        return discord_session_context_runtime.SessionContextRuntime(
            get_runtime_state=self.get_runtime_state,
            get_context_refresh_bridge=self.get_context_refresh_bridge,
            make_session_mirror_item_func=self.make_session_mirror_item,
            build_interactive_notice_func=self.build_interactive_notice_from_payload,
            extract_message_text_func=self.extract_message_text_from_payload,
            extract_user_text_func=self.extract_user_text_from_session_event,
            iter_recent_session_tail_events_func=self.iter_recent_session_tail_events,
            should_skip_discord_origin_prompt_func=self.should_skip_discord_origin_prompt,
            now=self.now,
            ttl_seconds=cast(float, getattr(self.module, "DISCORD_ORIGIN_PROMPT_TTL_SECONDS")),
            recent_prompt_dedupe_seconds=cast(float, getattr(self.module, "RECENT_CODEX_APP_PROMPT_DEDUPE_SECONDS")),
            recent_prompt_scan_bytes=cast(int, getattr(self.module, "RECENT_CODEX_APP_PROMPT_SCAN_BYTES")),
            context_refresh_default_limit=cast(int, getattr(self.module, "CONTEXT_REFRESH_DEFAULT_LIMIT")),
            context_refresh_max_limit=cast(int, getattr(self.module, "CONTEXT_REFRESH_MAX_LIMIT")),
            context_refresh_item_max_chars=cast(int, getattr(self.module, "CONTEXT_REFRESH_ITEM_MAX_CHARS")),
            session_mirror_recent_text_ttl_seconds=cast(
                float,
                getattr(self.module, "SESSION_MIRROR_RECENT_TEXT_TTL_SECONDS"),
            ),
            recent_prompt_expected_exceptions=self.recent_prompt_expected_exceptions(),
            format_exception=self.format_exception,
            log=self.log_line,
            get_thread_goal_lookup_func=self.get_thread_goal_lookup,
            get_thread_goal_update_func=self.get_thread_goal_update,
            get_thread_turn_completions_func=self.get_thread_turn_completions,
            get_session_mirror_detail_mode_func=detail_runtime.get_mode,
        )

    def get_thread_goal_lookup(self, thread_id: str) -> app_server_transport.ThreadGoalLookup:
        if not cast(Callable[[], bool], self._module_func("app_server_transport_enabled"))():
            return app_server_transport.GoalAbsent()
        return app_server_transport.DEFAULT_CLIENT.get_thread_goal_lookup(thread_id)

    def get_thread_goal_update(
        self,
        thread_id: str,
        turn_id: str,
    ) -> app_server_transport.ThreadGoalUpdate | None:
        return app_server_transport.DEFAULT_CLIENT.get_cached_goal_update(thread_id, turn_id)

    def get_thread_turn_completions(
        self,
        thread_id: str,
        turn_ids: list[str],
    ) -> dict[str, app_server_transport.TurnCompletion]:
        client = app_server_transport.DEFAULT_CLIENT
        requested_turn_ids = frozenset(turn_ids)
        cached = client.get_cached_thread_turn_completions(thread_id)
        if requested_turn_ids.issubset(cached):
            return cached
        if not client.is_running():
            return cached
        try:
            completions = client.get_thread_turn_completions(thread_id, timeout_sec=3.0)
        except TimeoutError:
            cached = client.get_cached_thread_turn_completions(thread_id)
            if requested_turn_ids.issubset(cached):
                return cached
            raise
        completions.update(client.get_cached_thread_turn_completions(thread_id))
        return completions

    def get_runtime_state(self) -> discord_runtime.DiscordRuntimeState:
        return cast(Callable[[], discord_runtime.DiscordRuntimeState], self._module_func("get_runtime_state"))()

    def get_context_refresh_bridge(self) -> discord_bridge_protocols.CodexBridgeContextRefreshModule:
        return cast(discord_bridge_protocols.CodexBridgeContextRefreshModule, getattr(self.module, "BRIDGE_CONTEXT_REFRESH"))

    def make_session_mirror_item(
        self,
        codex_thread_id: str,
        event: JsonEvent,
        *,
        kind: str,
        role: str,
        phase: str,
        text: str,
    ) -> dict[str, str]:
        return cast(
            Callable[..., dict[str, str]],
            self._module_func("make_session_mirror_item"),
        )(
            codex_thread_id,
            event,
            kind=kind,
            role=role,
            phase=phase,
            text=text,
        )

    def build_interactive_notice_from_payload(self, payload: Mapping[str, JsonValue]) -> str:
        return cast(
            Callable[[Mapping[str, JsonValue]], str],
            self._module_func("build_interactive_notice_from_payload"),
        )(payload)

    def extract_message_text_from_payload(self, payload: Mapping[str, JsonValue]) -> str:
        return cast(
            Callable[[Mapping[str, JsonValue]], str],
            self._module_func("extract_message_text_from_payload"),
        )(payload)

    def extract_user_text_from_session_event(self, event: JsonEvent) -> str:
        return cast(Callable[[JsonEvent], str], self._module_func("extract_user_text_from_session_event"))(event)

    def iter_recent_session_tail_events(
        self,
        session_path: Path,
        *,
        scan_bytes: int | None = None,
    ) -> list[JsonEvent]:
        return cast(
            Callable[..., list[JsonEvent]],
            self._module_func("iter_recent_session_tail_events"),
        )(session_path, scan_bytes=scan_bytes)

    def should_skip_discord_origin_prompt(self, target_thread_id: str | None, text: str) -> bool:
        return cast(
            Callable[[str | None, str], bool],
            self._module_func("should_skip_discord_origin_prompt"),
        )(target_thread_id, text)

    def now(self) -> float:
        time_module = cast(ModuleType, getattr(self.module, "time"))
        return cast(Callable[[], float], getattr(time_module, "monotonic"))()

    def recent_prompt_expected_exceptions(self) -> tuple[type[Exception], ...]:
        sqlite_module = cast(ModuleType, getattr(self.module, "sqlite3"))
        sqlite_error_type = cast(type[Exception], getattr(sqlite_module, "Error"))
        return (OSError, RuntimeError, sqlite_error_type)

    def format_exception(self) -> str:
        traceback_module = cast(ModuleType, getattr(self.module, "traceback"))
        return cast(Callable[[], str], getattr(traceback_module, "format_exc"))()

    def log_line(self, message: str) -> None:
        cast(Callable[[str], None], self._module_func("log_line"))(message)

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
