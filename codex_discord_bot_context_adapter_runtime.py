from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, cast

import codex_discord_context as discord_context
import codex_discord_context_refresh as discord_context_refresh
import codex_discord_context_status as discord_context_status
import codex_discord_weekly_usage as discord_weekly_usage
import codex_discord_where as discord_where
from codex_session_events import JsonEvent, JsonValue
from codex_thread_models import ThreadInfo


class IterRecentSessionTailEventsFunc(Protocol):
    def __call__(self, session_path: Path) -> list[JsonEvent]: ...


class CollectContextRefreshItemsFunc(Protocol):
    def __call__(self, target_thread_id: str, events: list[JsonEvent]) -> list[dict[str, str]]: ...


@dataclass(frozen=True, slots=True)
class BotContextAdapterRuntime:
    module: ModuleType

    def format_context_usage_line(self, thread: ThreadInfo) -> str:
        return discord_context.format_context_usage_line(
            thread,
            bridge_module=cast(discord_context_status.ContextStatusBridge, getattr(self.module, "BRIDGE_CONTEXT")),
        )

    def build_context_warning(self, target_thread_id: str | None) -> str:
        return discord_context.build_context_warning(
            target_thread_id,
            bridge_module=cast(discord_context_status.ContextStatusBridge, getattr(self.module, "BRIDGE_CONTEXT")),
            resolve_target_ref_func=cast(
                Callable[[str | None], tuple[str | None, str]],
                getattr(self.module, "resolve_target_ref"),
            ),
            log_func=cast(Callable[[str], None], getattr(self.module, "log_line")),
        )

    def build_context_message(
        self,
        channel_id: int | None = None,
        *,
        all_threads: bool = False,
        limit: int = 10,
    ) -> str:
        get_mirrored_thread = cast(Callable[[int | None], str | None], getattr(self.module, "get_mirrored_codex_thread_id"))
        resolve_selected = cast(Callable[[], tuple[str | None, str]], getattr(self.module, "resolve_selected_target"))
        return discord_context.build_context_message(
            channel_id,
            all_threads=all_threads,
            limit=limit,
            bridge_module=cast(discord_context_status.ContextStatusBridge, getattr(self.module, "BRIDGE_CONTEXT")),
            get_mirrored_codex_thread_id_func=get_mirrored_thread,
            resolve_selected_target_func=resolve_selected,
        )

    def build_context_refresh_message(
        self,
        channel_id: int | None = None,
        *,
        limit: int | None = None,
        max_chars: int | None = None,
    ) -> str:
        default_limit = cast(int, getattr(self.module, "CONTEXT_REFRESH_DEFAULT_LIMIT"))
        default_max_chars = cast(int, getattr(self.module, "CONTEXT_REFRESH_MAX_CHARS"))
        requested_limit = limit if limit is not None else default_limit
        requested_max_chars = max_chars if max_chars is not None else default_max_chars
        bounded_limit = cast(Callable[[int], int], getattr(self.module, "clamp_context_refresh_limit"))(requested_limit)
        return discord_context_refresh.build_context_refresh_message(
            channel_id,
            limit=bounded_limit,
            max_chars=requested_max_chars,
            bridge_module=cast(
                discord_context_refresh.ContextRefreshBridge,
                getattr(self.module, "BRIDGE_CONTEXT_REFRESH"),
            ),
            get_mirrored_codex_thread_id_func=cast(
                Callable[[int | None], str | None],
                getattr(self.module, "get_mirrored_codex_thread_id"),
            ),
            resolve_selected_target_func=cast(
                Callable[[], tuple[str | None, str]],
                getattr(self.module, "resolve_selected_target"),
            ),
            iter_recent_session_tail_events_func=cast(
                IterRecentSessionTailEventsFunc,
                getattr(self.module, "iter_recent_session_tail_events"),
            ),
            collect_context_refresh_items_func=cast(
                CollectContextRefreshItemsFunc,
                getattr(self.module, "collect_context_refresh_items"),
            ),
            format_context_refresh_item_func=cast(
                Callable[[dict[str, str]], str],
                getattr(self.module, "format_context_refresh_item"),
            ),
        )

    def format_weekly_usage_percent(self, value: JsonValue | None) -> str:
        formatter = cast(Callable[[JsonValue | None], str], getattr(self.module, "format_percent"))
        return formatter(value)

    def build_weekly_usage_message(self, days: int = 7) -> str:
        return discord_context.build_weekly_usage_message(
            days,
            bridge_module=cast(discord_weekly_usage.WeeklyUsageBridge, getattr(self.module, "BRIDGE_CONTEXT")),
            format_percent_func=self.format_weekly_usage_percent,
        )

    def build_where_message(self, channel_id: int | None) -> str:
        return discord_where.build_where_message(
            channel_id,
            bridge_module=cast(discord_where.WhereBridge, getattr(self.module, "BRIDGE_WHERE")),
            get_mirrored_codex_thread_id_func=cast(
                Callable[[int | None], str | None],
                getattr(self.module, "get_mirrored_codex_thread_id"),
            ),
            describe_mirrored_project_channel_func=cast(
                Callable[[int | None], str],
                getattr(self.module, "describe_mirrored_project_channel"),
            ),
            format_context_usage_line_func=self.format_context_usage_line,
        )
